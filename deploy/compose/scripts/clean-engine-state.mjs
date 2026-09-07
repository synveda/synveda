#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  closeSync,
  constants,
  existsSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readlinkSync,
  readSync,
  realpathSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";
import {
  ReceiptFailure,
  buildEnvironmentManifest,
  createFinalization,
  createNextReceipt,
  receiptFileName,
  validateReceiptChain,
} from "./clean-engine-receipts.mjs";
import {
  CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS,
  CONTROLLED_BACKGROUND_OPERATION_KIND,
  CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
  CONTROLLED_BACKGROUND_RETIREMENT_AUTHORITY_CHECKPOINTS,
  CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
  CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND,
  ProviderProcessContractFailure,
  inspectControlledBackgroundProvider,
  inspectControlledBackgroundProviderPrefix,
  inspectControlledBackgroundRetirementPrefix,
  launchControlledBackgroundProviderWithAuthorityGate,
  planControlledBackgroundProviderCreateWithAuthorityGate,
  planControlledBackgroundProviderOperation,
  planControlledBackgroundRetirementWithAuthorityGate,
  providerProcessBytes,
  providerProcessDigest,
  retireControlledBackgroundProviderWithAuthorityGate,
  validateControlledBackgroundProviderOperationPlan,
} from "./clean-engine-provider-process-contract.mjs";
import {
  COLIMA_LIVE_BASELINE_DESCRIPTOR_BINDING_FIELDS,
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_MAX_BASELINE_DESCENDANTS,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  ColimaLiveContractFailure,
  colimaLiveBytes,
  colimaLiveDigest,
  observeColimaLivePreEffectRoots,
  observeColimaLivePreEffectRootsForTest,
  observeColimaLiveProviderReservationRoots,
  observeColimaLiveProviderReservationRootsForTest,
} from "./clean-engine-colima-live-contract.mjs";
import {
  LiveProviderPlanFailure,
  buildColimaLiveProviderOperationPlan,
  liveProviderPlanBytes,
  liveProviderPlanDigest,
  validateColimaLiveProviderOperationPlan,
} from "./clean-engine-live-provider-plan.mjs";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
  COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
  COLIMA_LIVE_PROVIDER_INTENT_ACTION,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
  LiveProviderIntentFailure,
  buildColimaLiveEffectIntentCandidateStructure,
  buildColimaLiveEmptyPreEffectPrefixStructure,
  buildColimaLivePlanCompletionProjectionStructure,
  buildColimaLiveProviderIntentCompletion,
  buildColimaLiveProviderIntentPublicationPlan,
  liveProviderIntentBytes,
  validateColimaLiveProviderIntentPublicationPlan,
} from "./clean-engine-live-provider-intent.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
  LiveProviderStartDecisionFailure,
  buildColimaLiveCompletedProviderIntentProjectionStructure,
  buildColimaLiveProviderStartDecisionCompletion,
  buildColimaLiveProviderStartDecisionPublicationPlan,
  buildColimaLiveProviderStartFreshAdmissionStructure,
  liveProviderStartDecisionBytes,
  validateColimaLiveProviderStartDecisionPublicationPlan,
} from "./clean-engine-live-provider-start-decision.mjs";
import {
  LiveProviderProcessStartFailure,
  buildColimaLiveCompletedProviderStartDecisionProjectionStructure,
  buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
  liveProviderProcessStartBytes,
} from "./clean-engine-live-provider-process-start.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
  COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS,
  COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS,
  COLIMA_LIVE_PROVIDER_EFFECT_ROLES,
  LiveProviderEffectFailure,
  buildColimaLiveProviderEffectEvent,
  buildColimaLiveProviderEffectPublicationPlan,
  buildColimaLiveProviderEffectWitness,
  liveProviderEffectBytes,
  liveProviderEffectDigest,
  validateColimaLiveProviderEffectEventHistory,
  validateColimaLiveProviderEffectPublicationPlan,
  validateColimaLiveProviderEffectWitness,
} from "./clean-engine-live-provider-effect.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
  COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
  COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_KIND,
  LiveProviderReservationFailure,
  buildColimaLiveProviderReservationCompletion,
  buildColimaLiveProviderReservationPublicationPlan,
  buildColimaLiveProviderReservationSettlement,
  buildColimaLiveProviderReservationWitness,
  liveProviderReservationBytes,
  validateColimaLiveProviderReservationPublicationPlan,
  validateColimaLiveProviderReservationSettlement,
  validateColimaLiveProviderReservationWitness,
} from "./clean-engine-live-provider-reservation.mjs";

export {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
  COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
};

const REGISTRY_IMAGE =
  "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33";
const MAX_FILE_BYTES = 256 * 1024;
const MAX_CONTEXT_ENTRIES = 100_000;
const MAX_INERT_STAGING = 8;
const MAX_CONTEXT_FILE_BYTES = 128n * 1024n * 1024n;
const MAX_CONTEXT_TOTAL_BYTES = 2n * 1024n * 1024n * 1024n;
const OWNER_UID = BigInt(process.getuid());
const ZERO_SHA256 = "0".repeat(64);
const RECEIPT_STAGING_NAME = ".receipt-publish";
const ENVIRONMENT_NAME = "environment.json";
const ENVIRONMENT_STAGING_NAME = ".environment-publish";
const LEGACY_MUTATION_LEASE_NAME = ".mutation-lease";
const MUTATION_SLOT_PREFIX = ".mutation-slot-";
const MUTATION_CLOSE_PREFIX = ".mutation-close-";
const MUTATION_SLOT_SCHEMA = "synveda.clean-engine.mutation-slot.v7";
const MUTATION_CLOSE_SCHEMA = "synveda.clean-engine.mutation-close.v8";
const MUTATION_RECOVERY_PREFIX = ".mutation-recovery-";
const MUTATION_RECOVERY_SCHEMA = "synveda.clean-engine.mutation-recovery.v6";
const MUTATION_OPERATION_PREFIX = ".mutation-operation-";
const BACKGROUND_CREATE_SETTLEMENT_SCHEMA =
  "synveda.clean-engine.background-create-settlement.v1";
const BACKGROUND_CLEANUP_OPERATION_PLAN_SCHEMA =
  "synveda.clean-engine.background-cleanup-operation-plan.v1";
const BACKGROUND_CLEANUP_SETTLEMENT_SCHEMA =
  "synveda.clean-engine.background-cleanup-settlement.v1";
const MUTATION_STAGE_PREFIX = ".mutation-stage-";
const PROVIDER_RESERVATION_STAGE_PREFIX = ".provider-reservation-stage-";
const PROVIDER_RESERVATION_WITNESS_PREFIX = ".provider-reservation-witness-";
const PROVIDER_EFFECT_STAGE_PREFIX = ".provider-effect-stage-";
const PROVIDER_EFFECT_WITNESS_PREFIX = ".provider-effect-witness-";
const PROVIDER_EFFECT_EVENT_PREFIX = ".provider-effect-event-";
const PROVIDER_RESERVATION_RECOVERY_STAGES = Object.freeze({
  "marker-held": Object.freeze({
    disposition: "pending",
    links: 2,
    settlement: false,
  }),
  "marker-linked": Object.freeze({
    disposition: "pending",
    links: 2,
    settlement: false,
  }),
  "marker-reacquired": Object.freeze({
    disposition: "pending",
    links: 2,
    settlement: false,
  }),
  "not-started": Object.freeze({
    disposition: "not-reached",
    links: 0,
    settlement: false,
  }),
  "retirement-authorized-held": Object.freeze({
    disposition: "pending",
    links: 2,
    settlement: true,
  }),
  "retirement-authorized-retired": Object.freeze({
    disposition: "complete",
    links: 1,
    settlement: true,
  }),
  "stage-only": Object.freeze({
    disposition: "not-reached",
    links: 1,
    settlement: false,
  }),
  "stage-retired-before-effect": Object.freeze({
    disposition: "not-reached",
    links: 0,
    settlement: false,
  }),
  "witness-linked": Object.freeze({
    disposition: "pending",
    links: 3,
    settlement: false,
  }),
  "witness-unlinked": Object.freeze({
    disposition: "pending",
    links: 1,
    settlement: false,
  }),
});
const PROVIDER_RESERVATION_RECOVERY_TRANSITIONS = Object.freeze({
  "marker-held": Object.freeze([
    "marker-held",
    "retirement-authorized-held",
  ]),
  "marker-linked": Object.freeze(["marker-linked", "witness-linked"]),
  "marker-reacquired": Object.freeze([
    "marker-reacquired",
    "retirement-authorized-held",
  ]),
  "not-started": Object.freeze(["not-started"]),
  "retirement-authorized-held": Object.freeze([
    "retirement-authorized-held",
    "retirement-authorized-retired",
  ]),
  "retirement-authorized-retired": Object.freeze([
    "retirement-authorized-retired",
  ]),
  "stage-only": Object.freeze([
    "stage-only",
    "stage-retired-before-effect",
  ]),
  "stage-retired-before-effect": Object.freeze([
    "stage-retired-before-effect",
  ]),
  "witness-linked": Object.freeze(["marker-held", "witness-linked"]),
  "witness-unlinked": Object.freeze([
    "marker-reacquired",
    "witness-unlinked",
  ]),
});
const PROVIDER_RESERVATION_SETTLEMENT_AUTHORITY_STAGES = Object.freeze([
  "marker-held",
  "marker-linked",
  "marker-reacquired",
  "witness-linked",
  "witness-unlinked",
]);
const PROVIDER_EFFECT_RECOVERY_STAGES = Object.freeze({
  "attempt-fenced": Object.freeze({ disposition: "pending", links: 2 }),
  "authority-held": Object.freeze({ disposition: "pending", links: 2 }),
  "completion-retired": Object.freeze({ disposition: "complete", links: 1 }),
  "marker-held": Object.freeze({ disposition: "pending", links: 2 }),
  "marker-linked": Object.freeze({ disposition: "pending", links: 2 }),
  "not-started": Object.freeze({ disposition: "not-reached", links: 0 }),
  "stage-initializing": Object.freeze({
    disposition: "not-reached",
    links: 1,
  }),
  "stage-only": Object.freeze({ disposition: "not-reached", links: 1 }),
  "stage-retired-before-effect": Object.freeze({
    disposition: "not-reached",
    links: 0,
  }),
  "witness-linked": Object.freeze({ disposition: "pending", links: 3 }),
  "witness-unlinked": Object.freeze({ disposition: "pending", links: 1 }),
});
const PROVIDER_EFFECT_RECOVERY_TRANSITIONS = Object.freeze({
  "attempt-fenced": Object.freeze(["attempt-fenced"]),
  "authority-held": Object.freeze(["authority-held", "witness-unlinked"]),
  "completion-retired": Object.freeze(["completion-retired"]),
  "marker-held": Object.freeze(["marker-held", "witness-unlinked"]),
  "marker-linked": Object.freeze(["marker-linked", "witness-linked"]),
  "not-started": Object.freeze(["not-started"]),
  "stage-initializing": Object.freeze([
    "stage-initializing",
    "stage-retired-before-effect",
  ]),
  "stage-only": Object.freeze(["stage-only", "stage-retired-before-effect"]),
  "stage-retired-before-effect": Object.freeze([
    "stage-retired-before-effect",
  ]),
  "witness-linked": Object.freeze(["marker-held", "witness-linked"]),
  "witness-unlinked": Object.freeze([
    "completion-retired",
    "witness-unlinked",
  ]),
});
const MUTATION_OWNER_PROBE = "opaque-process-instance-v1";
const MAX_MUTATION_RECOVERIES = 8;
const MAX_MUTATION_SLOTS = 64;
const MAX_MUTATION_STAGES = 16;
const MAX_MUTATION_PUBLICATION_ATTEMPTS = 16;
const MAX_PLAN_RUN_INVENTORY_SUPERSESSIONS = 16;
const LIVE_PROVIDER_PLAN_ACTION = "provider-plan";
const DETERMINISTIC_PROVIDER_OPERATION_KIND = "deterministic-fake-provider-create-v1";
const FAKE_PROVIDER_ADAPTER_FIELDS = Object.freeze([
  "close_prelink_hold_milliseconds",
  "execute_outcome",
  "execute_result",
  "hold_milliseconds",
  "kind",
  "prelink_hold_milliseconds",
  "publication_hold_milliseconds",
  "reconcile_hold_milliseconds",
  "reconcile_outcome",
  "reconcile_result",
]);
const FAKE_PROVIDER_CONTRACT = Object.freeze({
  adapter_fields: FAKE_PROVIDER_ADAPTER_FIELDS,
  execute_outcomes: Object.freeze(["failed", "passed"]),
  kind: "deterministic-fake-provider-v1",
  max_hold_milliseconds: 30_000,
  reconcile_outcomes: Object.freeze(["failed", "passed", "unknown"]),
  result_contract: "clean-engine-provider-receipt-v6",
  schema: "synveda.clean-engine.fake-provider-contract.v1",
});
const FAKE_PROVIDER_CONTRACT_SHA256 = digest(canonicalBytes(FAKE_PROVIDER_CONTRACT));
const BACKGROUND_PROVIDER_ADAPTER_FIELDS = Object.freeze([
  "after_authority_hold_milliseconds",
  "after_evidence_hold_milliseconds",
  "after_intent_hold_milliseconds",
  "after_result_hold_milliseconds",
  "after_settlement_hold_milliseconds",
  "before_detach_hold_milliseconds",
  "before_identity_probe_hold_milliseconds",
  "before_start_decision_hold_milliseconds",
  "before_start_hold_milliseconds",
  "kind",
  "maximum_lifetime_milliseconds",
]);
const BACKGROUND_CLEANUP_ADAPTER_FIELDS = Object.freeze([
  "after_claim_hold_milliseconds",
  "after_intent_hold_milliseconds",
  "after_plan_hold_milliseconds",
  "after_result_hold_milliseconds",
  "after_retirement_hold_milliseconds",
  "after_slot_hold_milliseconds",
  "after_settlement_hold_milliseconds",
  "before_close_hold_milliseconds",
  "close_prelink_hold_milliseconds",
  "crash_after_delete_sequence",
  "crash_after_delete_syscall_sequence",
  "crash_after_hostagent_settlement",
  "kind",
  "stop_after_sequence",
]);
const REQUESTED_ASSERTIONS = Object.freeze([
  "browser-pkce-admin-logout-no-capture",
  "builder-canary-zero-connections",
  "canonical-proxy-values-empty",
  "clean-engine-initial-state",
  "disposable-engine-destroyed",
  "docker-client-proxy-active",
  "exact-local-embedded-builder",
  "exact-project-cleanup",
  "registry-auth-negative-positive",
  "source-closure-unchanged",
]);
const EXCLUDED_CLAIMS = Object.freeze([
  "disaster-recovery",
  "docker-desktop-parity",
  "enterprise-certification",
  "high-availability",
  "host-loss-tolerance",
  "native-linux-parity",
  "production-saas-readiness",
  "reference-https",
  "signed-provenance",
  "zero-downtime-upgrades",
]);
const PROXY_TEMPLATE = Object.freeze({
  auths: Object.freeze({}),
  proxies: Object.freeze({
    default: Object.freeze({
      allProxy: "socks5://all-proxy-canary.invalid:65535",
      ftpProxy: "http://ftp-proxy-canary.invalid:65535",
      httpProxy: "http://http-proxy-canary.invalid:65535",
      httpsProxy: "http://https-proxy-canary.invalid:65535",
      noProxy: "proxy-bypass-canary.invalid",
    }),
  }),
});
const DOCKERIGNORE_CONTRACT = Object.freeze([
  ".git",
  ".git/**",
  ".agents",
  ".agents/**",
  ".codex",
  ".codex/**",
  ".claude",
  ".claude/**",
  "target",
  "target/**",
  "**/target",
  "**/target/**",
  "node_modules",
  "node_modules/**",
  "**/node_modules",
  "**/node_modules/**",
  ".pnpm-store",
  ".pnpm-store/**",
  "**/dist",
  "**/dist/**",
  "**/dist-test",
  "**/dist-test/**",
  "data",
  "data/**",
  "evals/fixtures/longmemeval/longmemeval_*.json",
  "evals/fixtures/longmemeval/LICENSE",
  "evals/fixtures/longmemeval/LICENSE.*",
  ".env",
  ".env.*",
  "**/.env",
  "**/.env.*",
  "!**/.env.example",
  "deploy/compose/secrets",
  "deploy/compose/secrets/**",
  "deploy/compose/runtime",
  "deploy/compose/runtime/**",
  "deploy/compose/backups",
  "deploy/compose/backups/**",
  ".DS_Store",
  "**/.DS_Store",
  "Thumbs.db",
  "**/Thumbs.db",
  "*.log",
  "**/*.log",
  "*.tmp",
  "**/*.tmp",
]);

class ClosedFailure extends Error {
  constructor(code, status) {
    super(code);
    this.exitStatus = status;
  }
}

function fail(code, status = 78) {
  throw new ClosedFailure(code, status);
}

function parseArgs(argv) {
  const action = argv[2];
  if (!new Set(["plan", "status", "verify"]).has(action)) {
    fail("invalid action", 64);
  }
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
  const common = new Set(["repo-root", "state-base"]);
  const allowed = action === "plan" ? new Set([...common, "ipv4-pool", "provider"]) : common;
  if (
    Object.keys(values).some((name) => !allowed.has(name)) ||
    [...common].some((name) => values[name] === undefined) ||
    (action === "plan" && (values["ipv4-pool"] === undefined || values.provider === undefined))
  ) {
    fail("invalid arguments", 64);
  }
  return { action, values };
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
  fail("unsupported canonical value", 70);
}

function canonicalBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

function digest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function exactLstat(path) {
  return lstatSync(path, { bigint: true });
}

function exactFstat(descriptor) {
  return fstatSync(descriptor, { bigint: true });
}

function exactStat(path) {
  return statSync(path, { bigint: true });
}

function runGit(repoRoot, args, label) {
  const result = spawnSync("git", ["-C", repoRoot, ...args], {
    encoding: null,
    env: {
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_OPTIONAL_LOCKS: "0",
      GIT_TERMINAL_PROMPT: "0",
      LANG: "C",
      LC_ALL: "C",
      PATH: process.env.PATH ?? "/usr/bin:/bin",
    },
    maxBuffer: 16 * 1024 * 1024,
    timeout: 30_000,
    killSignal: "SIGKILL",
  });
  if (result.error !== undefined || result.status !== 0) fail(`${label} was unavailable`, 69);
  return result.stdout;
}

function onlyLowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function pathIsOutsideBuildContext(path) {
  if (path.includes("\0") || path.startsWith("/") || path.split("/").includes("..")) return false;
  const segments = path.split("/");
  const base = segments.at(-1);
  if ([".DS_Store", "Thumbs.db"].includes(base) || base.endsWith(".log") || base.endsWith(".tmp")) {
    return true;
  }
  if (base === ".env.example") return false;
  if (base === ".env" || base.startsWith(".env.")) return true;
  if (new Set([".git", "target", "node_modules", ".pnpm-store", "dist", "dist-test"]).has(segments[0])) {
    return true;
  }
  if (segments.some((segment) => new Set(["target", "node_modules", "dist", "dist-test"]).has(segment))) {
    return true;
  }
  if (new Set([".agents", ".claude", ".codex", "data"]).has(segments[0])) return true;
  if (
    segments.length === 4 &&
    segments[0] === "evals" &&
    segments[1] === "fixtures" &&
    segments[2] === "longmemeval" &&
    (segments[3] === "LICENSE" ||
      segments[3].startsWith("LICENSE.") ||
      /^longmemeval_.*\.json$/.test(segments[3]))
  ) {
    return true;
  }
  return (
    segments[0] === "deploy" &&
    segments[1] === "compose" &&
    new Set(["backups", "runtime", "secrets"]).has(segments[2])
  );
}

function dockerIgnoreContract(repoRoot) {
  let source;
  try {
    source = readFileSync(join(repoRoot, ".dockerignore"), "utf8");
  } catch {
    fail("Docker ignore contract was unavailable", 69);
  }
  const patterns = source
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"));
  if (
    new Set(patterns).size !== patterns.length ||
    JSON.stringify(patterns) !== JSON.stringify(DOCKERIGNORE_CONTRACT)
  ) {
    fail("Docker ignore contract was refused");
  }
}

function nulFields(buffer, label) {
  if (buffer.length === 0) return [];
  if (buffer.at(-1) !== 0) fail(`${label} was malformed`, 69);
  return buffer
    .subarray(0, -1)
    .toString("utf8")
    .split("\0");
}

function nulBufferFields(buffer, label) {
  if (buffer.length === 0) return [];
  if (buffer.at(-1) !== 0) fail(`${label} was malformed`, 69);
  const fields = [];
  let start = 0;
  for (let index = 0; index < buffer.length; index += 1) {
    if (buffer[index] !== 0) continue;
    fields.push(buffer.subarray(start, index));
    start = index + 1;
  }
  return fields;
}

function portablePath(bytes, label) {
  let path;
  try {
    path = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    fail(`${label} path was refused`, 69);
  }
  if (
    path === "" ||
    path.startsWith("/") ||
    /[\0-\x1f\x7f]/.test(path) ||
    path.split("/").some((part) => part === "" || part === "." || part === "..") ||
    !Buffer.from(path, "utf8").equals(bytes)
  ) {
    fail(`${label} path was refused`, 69);
  }
  return path;
}

function trackedEntries(indexBytes) {
  const entries = new Map();
  for (const field of nulBufferFields(indexBytes, "tracked source inventory")) {
    const separator = field.indexOf(0x09);
    if (separator < 1) fail("tracked source inventory was malformed", 69);
    const header = field.subarray(0, separator).toString("ascii");
    const match = header.match(/^([0-7]{6}) ([0-9a-f]{40}|[0-9a-f]{64}) ([0-3])$/);
    if (match === null || match[3] !== "0" || !new Set(["100644", "100755", "120000"]).has(match[1])) {
      fail("tracked source inventory was refused");
    }
    const path = portablePath(field.subarray(separator + 1), "tracked source");
    if (entries.has(path)) fail("tracked source inventory was refused");
    entries.set(path, { mode: match[1], object: match[2] });
  }
  if (entries.size === 0) fail("tracked source inventory was malformed", 69);
  return entries;
}

function sameMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.nlink === right.nlink &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

function modeString(metadata) {
  return (metadata.mode & 0o7777n).toString(8).padStart(4, "0");
}

function deploymentInput(path) {
  return (
    path === ".dockerignore" ||
    path === "Makefile" ||
    path === "deploy/compose" ||
    path.startsWith("deploy/compose/") ||
    path === "docs/DEPLOYMENT_CONTRACT.md" ||
    path === "docs/SECURITY.md"
  );
}

function readContextFile(path, initial, budget) {
  if (!initial.isFile() || initial.nlink !== 1n || initial.size > MAX_CONTEXT_FILE_BYTES) {
    fail("Docker build-context file was refused");
  }
  if (budget.bytes + initial.size > MAX_CONTEXT_TOTAL_BYTES) {
    fail("Docker build-context size was refused");
  }
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = exactFstat(descriptor);
    if (!sameMetadata(initial, before) || !before.isFile() || before.nlink !== 1n) {
      fail("Docker build-context file identity was refused");
    }
    const hash = createHash("sha256");
    const chunk = Buffer.allocUnsafe(64 * 1024);
    let offset = 0;
    while (offset < Number(before.size)) {
      const length = Math.min(chunk.length, Number(before.size) - offset);
      const count = readSync(descriptor, chunk, 0, length, offset);
      if (count < 1) fail("Docker build-context file changed while it was read");
      hash.update(chunk.subarray(0, count));
      offset += count;
    }
    const after = exactFstat(descriptor);
    const current = exactLstat(path);
    if (!sameMetadata(before, after) || !sameMetadata(after, current)) {
      fail("Docker build-context file changed while it was read");
    }
    budget.bytes += before.size;
    return { sha256: hash.digest("hex"), size: String(before.size) };
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail("Docker build-context file was unavailable", 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function readContextSymlink(path, initial, budget) {
  if (!initial.isSymbolicLink() || initial.nlink !== 1n) {
    fail("Docker build-context symlink was refused");
  }
  let target;
  try {
    target = readlinkSync(path, { encoding: "buffer" });
  } catch {
    fail("Docker build-context symlink was unavailable", 69);
  }
  const current = exactLstat(path);
  if (!sameMetadata(initial, current) || target.length > 16 * 1024) {
    fail("Docker build-context symlink changed while it was read");
  }
  if (budget.bytes + BigInt(target.length) > MAX_CONTEXT_TOTAL_BYTES) {
    fail("Docker build-context size was refused");
  }
  budget.bytes += BigInt(target.length);
  return { sha256: digest(target), size: String(target.length) };
}

function actualContextManifest(repoRoot, indexBytes) {
  const tracked = trackedEntries(indexBytes);
  const expectedDirectories = new Set();
  for (const path of tracked.keys()) {
    const parts = path.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const directory = parts.slice(0, index).join("/");
      if (pathIsOutsideBuildContext(directory)) break;
      expectedDirectories.add(directory);
    }
  }
  const seen = new Set();
  const contextHash = createHash("sha256");
  const deploymentHash = createHash("sha256");
  contextHash.update("synveda.docker-context-manifest.v1\0");
  deploymentHash.update("synveda.deployment-input-manifest.v1\0");
  const budget = { bytes: 0n, entries: 0 };
  const contractPath = "docs/DEPLOYMENT_CONTRACT.md";
  let contract;

  function record(path, value) {
    budget.entries += 1;
    if (budget.entries > MAX_CONTEXT_ENTRIES) fail("Docker build-context entry count was refused");
    const bytes = canonicalBytes({ path, ...value });
    contextHash.update(bytes);
    if (deploymentInput(path)) deploymentHash.update(bytes);
    if (path === contractPath) contract = value;
  }

  function walk(directory, relativeDirectory = "") {
    const initial = exactLstat(directory);
    if (!initial.isDirectory() || initial.isSymbolicLink()) {
      fail("Docker build-context directory was refused");
    }
    let names;
    try {
      names = readdirSync(directory, { encoding: "buffer" }).sort(Buffer.compare);
    } catch {
      fail("Docker build-context directory was unavailable", 69);
    }
    for (const nameBytes of names) {
      const name = portablePath(nameBytes, "Docker build-context");
      const path = relativeDirectory === "" ? name : `${relativeDirectory}/${name}`;
      if (pathIsOutsideBuildContext(path)) continue;
      const absolute = join(repoRoot, ...path.split("/"));
      const metadata = exactLstat(absolute);
      if (metadata.isDirectory() && !metadata.isSymbolicLink()) {
        if (!expectedDirectories.has(path)) fail("source worktree/context is not clean");
        record(path, { mode: modeString(metadata), type: "directory" });
        walk(absolute, path);
        continue;
      }
      const trackedEntry = tracked.get(path);
      if (trackedEntry === undefined) fail("source worktree/context is not clean");
      if (metadata.isFile() && trackedEntry.mode !== "120000") {
        const content = readContextFile(absolute, metadata, budget);
        record(path, { mode: modeString(metadata), type: "file", ...content });
      } else if (metadata.isSymbolicLink() && trackedEntry.mode === "120000") {
        const content = readContextSymlink(absolute, metadata, budget);
        record(path, { mode: modeString(metadata), type: "symlink", ...content });
      } else {
        fail("Docker build-context entry type was refused");
      }
      seen.add(path);
    }
    let finalNames;
    try {
      finalNames = readdirSync(directory, { encoding: "buffer" }).sort(Buffer.compare);
    } catch {
      fail("Docker build-context directory was unavailable", 69);
    }
    const current = exactLstat(directory);
    if (
      !sameMetadata(initial, current) ||
      names.length !== finalNames.length ||
      names.some((name, index) => !name.equals(finalNames[index]))
    ) {
      fail("Docker build-context directory changed while it was read");
    }
  }

  walk(repoRoot);
  for (const path of tracked.keys()) {
    if (!pathIsOutsideBuildContext(path) && !seen.has(path)) {
      fail("tracked Docker build-context input was unavailable", 69);
    }
  }
  if (
    contract === undefined ||
    contract.type !== "file" ||
    BigInt(contract.size) < 1n ||
    BigInt(contract.size) > 2n * 1024n * 1024n
  ) {
    fail("deployment contract size was refused", 69);
  }
  return {
    build_context_manifest_sha256: contextHash.digest("hex"),
    deployment_contract_sha256: contract.sha256,
    deployment_input_manifest_sha256: deploymentHash.digest("hex"),
  };
}

function sourceStatus(repoRoot) {
  return runGit(
    repoRoot,
    ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    "worktree inventory",
  );
}

function sourceClosure(repoRoot) {
  dockerIgnoreContract(repoRoot);
  const status = sourceStatus(repoRoot);
  if (status.length !== 0) fail("source worktree is not clean");

  const indexFlagBytes = runGit(repoRoot, ["ls-files", "-v", "-z"], "source index flags");
  const indexFlags = nulFields(indexFlagBytes, "source index flags");
  if (
    indexFlags.length === 0 ||
    indexFlags.some((entry) => entry.length < 3 || entry[0] !== "H" || entry[1] !== " ")
  ) {
    fail("source index contains hidden worktree state");
  }

  const ignored = nulFields(
    runGit(
      repoRoot,
      ["ls-files", "--others", "--ignored", "--exclude-standard", "-z"],
      "ignored source inventory",
    ),
    "ignored source inventory",
  );
  if (ignored.some((path) => !pathIsOutsideBuildContext(path))) {
    fail("ignored source input is not excluded from the build context");
  }

  const commitSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD"], "source commit")
    .toString("ascii")
    .trim();
  const treeSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD^{tree}"], "source tree")
    .toString("ascii")
    .trim();
  if (!onlyLowerHex(commitSha, 40) || !onlyLowerHex(treeSha, 40)) {
    fail("source identity was malformed", 69);
  }

  const buildInputs = runGit(repoRoot, ["ls-files", "--stage", "-z"], "tracked source inventory");
  const actual = actualContextManifest(repoRoot, buildInputs);
  const finalStatus = sourceStatus(repoRoot);
  const finalIndexFlags = runGit(repoRoot, ["ls-files", "-v", "-z"], "source index flags");
  const finalBuildInputs = runGit(
    repoRoot,
    ["ls-files", "--stage", "-z"],
    "tracked source inventory",
  );
  const finalCommitSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD"], "source commit")
    .toString("ascii")
    .trim();
  const finalTreeSha = runGit(repoRoot, ["rev-parse", "--verify", "HEAD^{tree}"], "source tree")
    .toString("ascii")
    .trim();
  const finalActual = actualContextManifest(repoRoot, finalBuildInputs);
  if (
    finalStatus.length !== 0 ||
    !finalIndexFlags.equals(indexFlagBytes) ||
    !finalBuildInputs.equals(buildInputs) ||
    finalCommitSha !== commitSha ||
    finalTreeSha !== treeSha ||
    canonical(finalActual) !== canonical(actual)
  ) {
    fail("source closure changed while it was read");
  }
  return {
    ...actual,
    commit_sha: commitSha,
    tracked_index_manifest_sha256: digest(buildInputs),
    tree_sha: treeSha,
    worktree_clean: true,
  };
}

function privateIpv4Pool(value) {
  if (typeof value !== "string") return false;
  const match = value.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.0\/24$/);
  if (match === null) return false;
  const rawOctets = match.slice(1);
  const octets = rawOctets.map(Number);
  if (octets.some((octet, index) => octet > 255 || String(octet) !== rawOctets[index])) return false;
  const [first, second] = octets;
  return first === 10 || (first === 172 && second >= 16 && second <= 31) ||
    (first === 192 && second === 168);
}

function inside(parent, candidate) {
  const path = relative(parent, candidate);
  return path === "" || (!path.startsWith(`..${sep}`) && path !== "..");
}

function ownedPrivateDirectory(path, label) {
  let metadata;
  try {
    metadata = exactLstat(path);
  } catch {
    fail(`${label} was unavailable`, 69);
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== OWNER_UID ||
    (metadata.mode & 0o7777n) !== 0o700n ||
    realpathSync(path) !== resolve(path)
  ) {
    fail(`${label} was refused`);
  }
  return metadata;
}

function prepareRoots(repoArgument, stateArgument, createState) {
  if (!isAbsolute(repoArgument) || !isAbsolute(stateArgument)) fail("paths must be absolute", 64);
  let repoRoot;
  try {
    repoRoot = realpathSync(repoArgument);
  } catch {
    fail("repository root was unavailable", 69);
  }
  if (repoRoot !== resolve(repoArgument) || !exactStat(repoRoot).isDirectory()) {
    fail("repository root was refused");
  }
  const stateBase = resolve(stateArgument);
  if (inside(repoRoot, stateBase) || inside(stateBase, repoRoot)) {
    fail("state base must be outside the repository");
  }
  if (!existsSync(stateBase)) {
    if (!createState) fail("state base was unavailable", 69);
    try {
      const missing = [];
      let ancestor = stateBase;
      while (!existsSync(ancestor)) {
        missing.unshift(ancestor);
        const parent = dirname(ancestor);
        if (parent === ancestor) fail("state base ancestor was unavailable", 69);
        ancestor = parent;
      }
      const ancestorMetadata = exactLstat(ancestor);
      if (
        !ancestorMetadata.isDirectory() ||
        ancestorMetadata.isSymbolicLink() ||
        realpathSync(ancestor) !== resolve(ancestor)
      ) {
        fail("state base ancestor was refused");
      }
      for (const directory of missing) {
        mkdirSync(directory, { mode: 0o700 });
        ownedPrivateDirectory(directory, "created state directory");
        syncDirectory(dirname(directory));
      }
    } catch {
      fail("state base creation failed", 70);
    }
  }
  ownedPrivateDirectory(stateBase, "state base");
  return { repoRoot, stateBase, active: join(stateBase, "active") };
}

function syncDirectory(path) {
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_DIRECTORY);
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

function writeExclusive(path, bytes, mode = 0o600) {
  let descriptor;
  try {
    descriptor = openSync(
      path,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      mode,
    );
    let offset = 0;
    while (offset < bytes.length) {
      const written = writeSync(descriptor, bytes, offset, bytes.length - offset);
      if (written < 1) fail("immutable state publication failed", 70);
      offset += written;
    }
    fsyncSync(descriptor);
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail("immutable state publication failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  syncDirectory(dirname(path));
}

function readPrivate(path, label, expectedLinks = 1, minimumBytes = 2n) {
  let descriptor;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = exactFstat(descriptor);
    if (
      !before.isFile() ||
      before.uid !== OWNER_UID ||
      before.nlink !== BigInt(expectedLinks) ||
      (before.mode & 0o7777n) !== 0o600n ||
      before.size < minimumBytes ||
      before.size > BigInt(MAX_FILE_BYTES)
    ) {
      fail(`${label} file was refused`);
    }
    const bytes = readFileSync(descriptor);
    const after = exactFstat(descriptor);
    if (
      after.dev !== before.dev ||
      after.ino !== before.ino ||
      after.size !== before.size ||
      after.mode !== before.mode ||
      after.nlink !== BigInt(expectedLinks)
    ) {
      fail(`${label} file changed while it was read`);
    }
    return bytes;
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail(`${label} file was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function parseCanonical(path, label, expectedLinks = 1, minimumBytes = 2n) {
  const bytes = readPrivate(path, label, expectedLinks, minimumBytes);
  let value;
  try {
    value = JSON.parse(bytes.toString("utf8"));
  } catch {
    fail(`${label} was not canonical JSON`);
  }
  if (!Buffer.from(canonicalBytes(value)).equals(bytes)) fail(`${label} was not canonical JSON`);
  return { bytes, value };
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") fail(`${label} was malformed`);
  if (JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    fail(`${label} fields were refused`);
  }
}

let currentProcessIdentityValue;

function currentBootSessionSha256() {
  let value;
  if (process.platform === "linux") {
    try {
      value = readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim();
    } catch {
      fail("process boot identity was unavailable", 69);
    }
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value)) {
      fail("process boot identity was refused", 69);
    }
  } else if (process.platform === "darwin") {
    let result = spawnSync("/usr/sbin/sysctl", ["-n", "kern.boottime"], {
      encoding: "utf8",
      env: { LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin:/usr/sbin:/sbin", TZ: "UTC0" },
      maxBuffer: 4096,
      timeout: 5_000,
    });
    value = result.stdout?.trim();
    if (
      result.error !== undefined ||
      result.status !== 0 ||
      typeof value !== "string" ||
      !/^\{ sec = [0-9]+, usec = [0-9]+ \} .+$/.test(value)
    ) {
      // Sandboxed macOS runners can deny both kernel boot and process-table
      // probes. The closed fallback is only an opaque platform label. Recovery
      // never treats a boot-label mismatch as death and still requires the
      // recorded PID to be absent, so this restriction can only make recovery
      // more conservative.
      value = "restricted-kernel-probe-unavailable";
    }
  } else {
    fail("process identity platform was unsupported", 69);
  }
  return digest(Buffer.from(`synveda.clean-engine.boot.v1\0${process.platform}\0${value}`, "utf8"));
}

function currentProcessIdentity() {
  if (currentProcessIdentityValue !== undefined) return currentProcessIdentityValue;
  const bootSha256 = currentBootSessionSha256();
  const instanceSha256 = digest(
    canonicalBytes({
      boot_sha256: bootSha256,
      nonce: randomBytes(32).toString("hex"),
      pid: process.pid,
      schema: "synveda.clean-engine.process-instance.v1",
    }),
  );
  currentProcessIdentityValue = Object.freeze({
    boot_sha256: bootSha256,
    instance_sha256: instanceSha256,
    pid: process.pid,
    probe: MUTATION_OWNER_PROBE,
  });
  return currentProcessIdentityValue;
}

function validateMutationOwner(value, prefix, label) {
  if (
    !onlyLowerHex(value[`${prefix}_boot_sha256`], 64) ||
    !onlyLowerHex(value[`${prefix}_instance_sha256`], 64) ||
    !Number.isSafeInteger(value[`${prefix}_pid`]) ||
    value[`${prefix}_pid`] < 1 ||
    value[`${prefix}_probe`] !== MUTATION_OWNER_PROBE
  ) {
    fail(`${label} owner identity was refused`);
  }
}

function operationPlanSha256(value) {
  return value === null ? ZERO_SHA256 : providerProcessDigest(providerProcessBytes(value));
}

function backgroundCreateBindings(slot) {
  if (slot.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND) {
    fail("background provider mutation operation was refused", 70);
  }
  return Object.freeze({
    create_intent_sha256: slot.value.intent_receipt_sha256,
    create_slot_sequence: slot.value.journal_sequence,
    create_slot_sha256: digest(slot.bytes),
    ownership_nonce: slot.value.operation_plan.ownership_nonce,
    source_head_sha256: slot.value.source_head_sha256,
    source_sequence: slot.value.source_sequence,
    state_integration: "mutation-journal-v2",
  });
}

function backgroundCleanupBindings(slot) {
  if (
    slot.value.action !== "provider-cleanup" ||
    slot.value.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
  ) {
    fail("background cleanup mutation operation was refused", 70);
  }
  const operationPlan = slot.value.operation_plan;
  return Object.freeze({
    cleanup_intent_sha256: slot.value.intent_receipt_sha256,
    cleanup_operation_plan_sha256: operationPlanSha256(operationPlan),
    cleanup_slot_sequence: slot.value.journal_sequence,
    cleanup_slot_sha256: digest(slot.bytes),
    create_close_sha256: operationPlan.create_close_sha256,
    create_settlement_sha256: operationPlan.create_settlement_sha256,
    create_slot_sha256: operationPlan.create_slot_sha256,
    source_head_sha256: slot.value.source_head_sha256,
    source_sequence: slot.value.source_sequence,
  });
}

function validateBoundDirectoryIdentity(value, label) {
  exactKeys(value, ["device", "inode", "mode", "path", "uid"], label);
  if (
    typeof value.path !== "string" ||
    !isAbsolute(value.path) ||
    resolve(value.path) !== value.path ||
    value.mode !== "0700" ||
    value.uid !== String(OWNER_UID) ||
    typeof value.device !== "string" ||
    !/^[0-9]+$/.test(value.device) ||
    typeof value.inode !== "string" ||
    !/^[0-9]+$/.test(value.inode)
  ) {
    fail(`${label} was refused`, 70);
  }
}

function currentBoundDirectoryIdentity(path, label) {
  const metadata = ownedPrivateDirectory(path, label);
  return Object.freeze({
    device: String(metadata.dev),
    inode: String(metadata.ino),
    mode: modeString(metadata),
    path,
    uid: String(metadata.uid),
  });
}

function validateBackgroundCleanupOperationPlan(value) {
  exactKeys(
    value,
    [
      "create_close_sha256",
      "create_operation_plan_sha256",
      "create_settlement_sha256",
      "create_slot_sequence",
      "create_slot_sha256",
      "evidence_directory",
      "fixture_id",
      "provider_base",
      "provider_contract_sha256",
      "provider_identity_sha256",
      "provider_kind",
      "provider_resource",
      "provider_root_key",
      "retirement_contract_sha256",
      "schema",
      "state_integration",
    ],
    "background cleanup operation plan",
  );
  validateBoundDirectoryIdentity(
    value.evidence_directory,
    "background cleanup evidence directory",
  );
  validateBoundDirectoryIdentity(value.provider_base, "background cleanup provider base");
  if (
    value.schema !== BACKGROUND_CLEANUP_OPERATION_PLAN_SCHEMA ||
    !onlyLowerHex(value.fixture_id, 32) ||
    value.create_slot_sequence !== 0 ||
    !onlyLowerHex(value.create_slot_sha256, 64) ||
    value.create_slot_sha256 === ZERO_SHA256 ||
    !onlyLowerHex(value.create_operation_plan_sha256, 64) ||
    value.create_operation_plan_sha256 === ZERO_SHA256 ||
    !onlyLowerHex(value.create_settlement_sha256, 64) ||
    value.create_settlement_sha256 === ZERO_SHA256 ||
    !onlyLowerHex(value.create_close_sha256, 64) ||
    value.create_close_sha256 === ZERO_SHA256 ||
    value.provider_contract_sha256 !==
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    !onlyLowerHex(value.provider_identity_sha256, 64) ||
    value.provider_identity_sha256 === ZERO_SHA256 ||
    value.provider_kind !== "controlled-background-fake" ||
    value.provider_resource !== `synveda-cpr45-${value.fixture_id}` ||
    value.provider_root_key !== `svb-${value.fixture_id.slice(0, 12)}` ||
    value.retirement_contract_sha256 !==
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 ||
    value.state_integration !== "mutation-journal-v2"
  ) {
    fail("background cleanup operation plan was refused", 70);
  }
}

function backgroundCleanupOperationPlan({
  createClose,
  createEvidence,
  createSettlement,
  createSlot,
}) {
  if (
    createClose === undefined ||
    createSettlement === undefined ||
    createSlot === undefined ||
    createSlot.value.action !== "provider-create" ||
    createSlot.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
    createClose.value.disposition !== "completed" ||
    createClose.value.operation_evidence_sha256 !== digest(createSettlement.bytes) ||
    createSettlement.value.disposition !== "complete-identity" ||
    createSettlement.value.safe_code !== "none" ||
    createEvidence?.providerIdentity?.sha256 !==
      createSettlement.value.evidence_head_sha256
  ) {
    fail("background cleanup create authority was refused", 73);
  }
  const createPlan = createSlot.value.operation_plan;
  const value = Object.freeze({
    create_close_sha256: digest(createClose.bytes),
    create_operation_plan_sha256: operationPlanSha256(createPlan),
    create_settlement_sha256: digest(createSettlement.bytes),
    create_slot_sequence: createSlot.value.journal_sequence,
    create_slot_sha256: digest(createSlot.bytes),
    evidence_directory: createPlan.evidence_directory,
    fixture_id: createSlot.value.fixture_id,
    provider_base: createPlan.provider_base,
    provider_contract_sha256: createSlot.value.operation_contract_sha256,
    provider_identity_sha256: createEvidence.providerIdentity.sha256,
    provider_kind: "controlled-background-fake",
    provider_resource: createPlan.provider_resource,
    provider_root_key: createPlan.provider_root_key,
    retirement_contract_sha256:
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    schema: BACKGROUND_CLEANUP_OPERATION_PLAN_SCHEMA,
    state_integration: "mutation-journal-v2",
  });
  validateBackgroundCleanupOperationPlan(value);
  return value;
}

function liveProviderIntentFixtureOnly(operationKind, contractSha256) {
  if (
    operationKind === COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND &&
    contractSha256 === COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256
  ) {
    return false;
  }
  if (
    operationKind === COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND &&
    contractSha256 ===
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256
  ) {
    return true;
  }
  fail("live provider intent operation contract was refused");
}

function validateLiveProviderIntentPublicationPlanForState(
  value,
  fixtureOnly,
) {
  try {
    return validateColimaLiveProviderIntentPublicationPlan(value, {
      fixtureOnly,
    });
  } catch (error) {
    if (error instanceof LiveProviderIntentFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function liveProviderStartDecisionFixtureOnly(operationKind, contractSha256) {
  if (
    operationKind === COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND &&
    contractSha256 ===
      COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256
  ) {
    return false;
  }
  if (
    operationKind ===
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND &&
    contractSha256 ===
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256
  ) {
    return true;
  }
  fail("live provider start decision operation contract was refused");
}

function liveProviderReservationFixtureOnly(operationKind, contractSha256) {
  if (
    operationKind === COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_KIND &&
    contractSha256 ===
      COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256
  ) {
    return false;
  }
  if (
    operationKind ===
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_KIND &&
    contractSha256 ===
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256
  ) {
    return true;
  }
  fail("live provider reservation operation contract was refused");
}

function liveProviderEffectFixtureOnly(operationKind, contractSha256) {
  if (
    operationKind === COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND &&
    contractSha256 ===
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256
  ) {
    return true;
  }
  fail("live provider effect operation contract was refused");
}

function validateLiveProviderEffectPublicationPlanForState(value, source) {
  try {
    return validateColimaLiveProviderEffectPublicationPlan(value, source);
  } catch (error) {
    if (error instanceof LiveProviderEffectFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function validateLiveProviderReservationPublicationPlanForState(
  value,
  source,
) {
  try {
    return validateColimaLiveProviderReservationPublicationPlan(value, source);
  } catch (error) {
    if (error instanceof LiveProviderReservationFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function validateLiveProviderStartDecisionPublicationPlanForState(
  value,
  source,
) {
  try {
    return validateColimaLiveProviderStartDecisionPublicationPlan(value, source);
  } catch (error) {
    if (error instanceof LiveProviderStartDecisionFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function validateMutationOperation(value) {
  if (value.action === LIVE_PROVIDER_PLAN_ACTION) {
    if (
      value.operation_plan === null ||
      Array.isArray(value.operation_plan) ||
      typeof value.operation_plan !== "object"
    ) {
      fail("live provider planning operation was refused");
    }
    try {
      validateColimaLiveProviderOperationPlan(value.operation_plan);
    } catch (error) {
      if (error instanceof LiveProviderPlanFailure) {
        fail(error.message, error.exitStatus);
      }
      throw error;
    }
    if (
      value.operation_kind !== value.operation_plan.operation_kind ||
      value.operation_contract_sha256 !==
        value.operation_plan.operation_contract_sha256 ||
      value.operation_plan.fixture_id !== value.fixture_id
    ) {
      fail("live provider planning operation binding was refused");
    }
    return;
  }
  if (value.action === COLIMA_LIVE_PROVIDER_INTENT_ACTION) {
    if (
      value.operation_plan === null ||
      Array.isArray(value.operation_plan) ||
      typeof value.operation_plan !== "object"
    ) {
      fail("live provider intent operation was refused");
    }
    const fixtureOnly = liveProviderIntentFixtureOnly(
      value.operation_kind,
      value.operation_contract_sha256,
    );
    validateLiveProviderIntentPublicationPlanForState(
      value.operation_plan,
      fixtureOnly,
    );
    if (
      value.operation_plan.fixture_id !== value.fixture_id ||
      value.operation_plan.operation_kind !== value.operation_kind ||
      value.operation_plan.operation_contract_sha256 !==
        value.operation_contract_sha256
    ) {
      fail("live provider intent operation binding was refused");
    }
    return;
  }
  if (value.action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION) {
    if (
      value.operation_plan === null ||
      Array.isArray(value.operation_plan) ||
      typeof value.operation_plan !== "object"
    ) {
      fail("live provider start decision operation was refused");
    }
    liveProviderStartDecisionFixtureOnly(
      value.operation_kind,
      value.operation_contract_sha256,
    );
    if (
      value.operation_plan.fixture_id !== value.fixture_id ||
      value.operation_plan.operation_kind !== value.operation_kind ||
      value.operation_plan.operation_contract_sha256 !==
        value.operation_contract_sha256
    ) {
      fail("live provider start decision operation binding was refused");
    }
    return;
  }
  if (value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION) {
    if (
      value.operation_plan === null ||
      Array.isArray(value.operation_plan) ||
      typeof value.operation_plan !== "object"
    ) {
      fail("live provider reservation operation was refused");
    }
    liveProviderReservationFixtureOnly(
      value.operation_kind,
      value.operation_contract_sha256,
    );
    if (
      value.operation_plan.fixture_id !== value.fixture_id ||
      value.operation_plan.operation_kind !== value.operation_kind ||
      value.operation_plan.operation_contract_sha256 !==
        value.operation_contract_sha256
    ) {
      fail("live provider reservation operation binding was refused");
    }
    return;
  }
  if (value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION) {
    if (
      value.operation_plan === null ||
      Array.isArray(value.operation_plan) ||
      typeof value.operation_plan !== "object"
    ) {
      fail("live provider effect operation was refused");
    }
    if (
      liveProviderEffectFixtureOnly(
        value.operation_kind,
        value.operation_contract_sha256,
      ) !== true ||
      value.operation_plan.fixture_id !== value.fixture_id ||
      value.operation_plan.operation_kind !== value.operation_kind ||
      value.operation_plan.operation_contract_sha256 !==
        value.operation_contract_sha256
    ) {
      fail("live provider effect operation binding was refused");
    }
    return;
  }
  if (value.action === "provider-create") {
    if (
      value.operation_kind === DETERMINISTIC_PROVIDER_OPERATION_KIND &&
      value.operation_contract_sha256 === FAKE_PROVIDER_CONTRACT_SHA256 &&
      value.operation_plan === null
    ) {
      return;
    }
    if (
      value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
      value.operation_contract_sha256 !==
        CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
      value.operation_plan === null ||
      Array.isArray(value.operation_plan) ||
      typeof value.operation_plan !== "object"
    ) {
      fail("provider mutation operation was refused");
    }
    try {
      validateControlledBackgroundProviderOperationPlan(value.operation_plan);
    } catch (error) {
      if (error instanceof ProviderProcessContractFailure) fail(error.message, error.exitStatus);
      throw error;
    }
    if (
      value.operation_plan.fixture_id !== value.fixture_id ||
      value.operation_plan.provider_contract_sha256 !== value.operation_contract_sha256
    ) {
      fail("provider mutation operation binding was refused");
    }
    return;
  }
  if (
    value.action === "provider-cleanup" &&
    value.operation_kind === CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
    value.operation_contract_sha256 ===
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 &&
    value.operation_plan !== null &&
    !Array.isArray(value.operation_plan) &&
    typeof value.operation_plan === "object"
  ) {
    validateBackgroundCleanupOperationPlan(value.operation_plan);
    if (
      value.operation_plan.fixture_id !== value.fixture_id ||
      value.operation_plan.retirement_contract_sha256 !==
        value.operation_contract_sha256
    ) {
      fail("background cleanup mutation operation binding was refused");
    }
    return;
  }
  if (
    value.operation_kind !== "none" ||
    value.operation_contract_sha256 !== ZERO_SHA256 ||
    value.operation_plan !== null
  ) {
    fail("journal-only mutation operation was refused");
  }
}

function validateMutationLeaseValue(value, fixtureId) {
  exactKeys(
    value,
    [
      "action",
      "fixture_id",
      "intent_receipt_sha256",
      "journal_sequence",
      "nonce",
      "operation_contract_sha256",
      "operation_kind",
      "operation_plan",
      "owner_boot_sha256",
      "owner_instance_sha256",
      "owner_pid",
      "owner_probe",
      "previous_close_sha256",
      "schema",
      "source_environment_sha256",
      "source_head_sha256",
      "source_sequence",
    ],
    "mutation slot",
  );
  if (
    value.schema !== MUTATION_SLOT_SCHEMA ||
    value.fixture_id !== fixtureId ||
    !new Set([
      "append-receipt",
      "finalize-environment",
      "provider-cleanup",
      "provider-create",
      COLIMA_LIVE_PROVIDER_INTENT_ACTION,
      COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
      COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
      COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
      LIVE_PROVIDER_PLAN_ACTION,
    ]).has(value.action) ||
    !onlyLowerHex(value.intent_receipt_sha256, 64) ||
    !Number.isSafeInteger(value.journal_sequence) ||
    value.journal_sequence < 0 ||
    value.journal_sequence >= MAX_MUTATION_SLOTS ||
    !onlyLowerHex(value.nonce, 32) ||
    !onlyLowerHex(value.operation_contract_sha256, 64) ||
    !onlyLowerHex(value.previous_close_sha256, 64) ||
    !onlyLowerHex(value.source_environment_sha256, 64) ||
    !onlyLowerHex(value.source_head_sha256, 64) ||
    !Number.isSafeInteger(value.source_sequence) ||
    value.source_sequence < 0 ||
    value.source_sequence > 63
  ) {
    fail("mutation slot was refused");
  }
  validateMutationOwner(value, "owner", "mutation slot");
  validateMutationOperation(value);
}

function mutationOwnerState(value, probe = defaultMutationOwnerProbe) {
  const state = probe({
    boot_sha256: value.owner_boot_sha256,
    instance_sha256: value.owner_instance_sha256,
    pid: value.owner_pid,
    probe: value.owner_probe,
  });
  if (!new Set(["absent", "current", "pid-reused", "unknown", "zombie"]).has(state)) {
    fail("mutation owner probe was refused", 70);
  }
  return state;
}

function defaultMutationOwnerProbe(owner) {
  let current;
  try {
    current = currentProcessIdentity();
  } catch {
    return "unknown";
  }
  if (owner.pid === current.pid) {
    return owner.boot_sha256 === current.boot_sha256 &&
      owner.instance_sha256 === current.instance_sha256
      ? "current"
      : "unknown";
  }
  try {
    process.kill(owner.pid, 0);
    // Another live PID cannot prove the opaque process-instance challenge. A
    // reused PID therefore blocks safely instead of being treated as the old
    // executor or reclaimed from a coarse timestamp.
    return "unknown";
  } catch (error) {
    return error?.code === "ESRCH" ? "absent" : "unknown";
  }
}

function mutationSlotFileName(sequence) {
  return `${MUTATION_SLOT_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function mutationCloseFileName(sequence) {
  return `${MUTATION_CLOSE_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function mutationOperationFileName(sequence) {
  return `${MUTATION_OPERATION_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function providerReservationWitnessFileName(sequence) {
  return `${PROVIDER_RESERVATION_WITNESS_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function providerEffectWitnessFileName(sequence) {
  return `${PROVIDER_EFFECT_WITNESS_PREFIX}${String(sequence).padStart(2, "0")}`;
}

function providerEffectStageFileName(sequence, nonce) {
  if (
    !Number.isSafeInteger(sequence) ||
    sequence < 0 ||
    sequence >= MAX_MUTATION_SLOTS ||
    !onlyLowerHex(nonce, 32)
  ) {
    fail("provider effect stage name was refused", 70);
  }
  return `${PROVIDER_EFFECT_STAGE_PREFIX}${String(sequence).padStart(2, "0")}-${nonce}`;
}

function providerEffectEventFileName(slotSequence, eventSequence) {
  return `${PROVIDER_EFFECT_EVENT_PREFIX}${String(slotSequence).padStart(2, "0")}-${String(eventSequence).padStart(3, "0")}`;
}

function recoveryFileName(slotSequence, sequence) {
  return `${MUTATION_RECOVERY_PREFIX}${String(slotSequence).padStart(2, "0")}-${String(sequence).padStart(2, "0")}`;
}

function recoveryChainRootSha256(fixtureId, leaseSha256, operation) {
  return digest(canonicalBytes({
    action: operation.action,
    fixture_id: fixtureId,
    lease_sha256: leaseSha256,
    operation_contract_sha256: operation.operation_contract_sha256,
    operation_kind: operation.operation_kind,
    operation_plan_sha256: operationPlanSha256(operation.operation_plan),
    schema: "synveda.clean-engine.mutation-recovery-root.v6",
  }));
}

function reservationRecoveryTopologySha256({
  evidenceSha256,
  evidenceStage,
  localLinks,
  settlementSha256,
  slotSequence,
}) {
  return digest(canonicalBytes({
    evidence_sha256: evidenceSha256,
    evidence_stage: evidenceStage,
    local_links: localLinks,
    settlement_sha256: settlementSha256,
    slot_sequence: slotSequence,
  }));
}

function effectRecoveryTopologySha256({
  eventCount,
  eventHeadSha256,
  evidenceStage,
  localLinks,
  markerPresent,
  slotSequence,
  witnessSha256,
}) {
  return digest(canonicalBytes({
    event_count: eventCount,
    event_head_sha256: eventHeadSha256,
    evidence_stage: evidenceStage,
    local_links: localLinks,
    marker_present: markerPresent,
    slot_sequence: slotSequence,
    witness_sha256: witnessSha256,
  }));
}

function liveProviderEffectRecoveryStageReachable(from, to) {
  const visited = new Set();
  const pending = [from];
  while (pending.length > 0) {
    const current = pending.shift();
    if (current === to) return true;
    if (visited.has(current)) continue;
    visited.add(current);
    for (const next of PROVIDER_EFFECT_RECOVERY_TRANSITIONS[current] ?? []) {
      if (!visited.has(next)) pending.push(next);
    }
  }
  return false;
}

function validateRecoveryClaims(claims, fixtureId, slot, operation) {
  const leaseSha256 = digest(slot.bytes);
  let previous;
  for (const claim of claims) {
    exactKeys(
      claim.value,
      [
        "action",
        "chain_root_sha256",
        "fixture_id",
        "lease_sha256",
        "nonce",
        "observed_effect_disposition",
        "observed_effect_name",
        "observed_evidence_head_sha256",
        "observed_evidence_prefix_sha256",
        "observed_evidence_stage",
        "observed_residual_sha256",
        "observed_settlement_sha256",
        "operation_contract_sha256",
        "operation_kind",
        "operation_plan_sha256",
        "owner_boot_sha256",
        "owner_instance_sha256",
        "owner_pid",
        "owner_probe",
        "parent_sha256",
        "schema",
        "sequence",
        "slot_sequence",
        "source_head_sha256",
      ],
      "mutation recovery claim",
    );
    if (
      claim.name !== recoveryFileName(claim.value.slot_sequence, claim.value.sequence) ||
      claim.value.schema !== MUTATION_RECOVERY_SCHEMA ||
      !Number.isSafeInteger(claim.value.sequence) ||
      claim.value.sequence < 0 ||
      claim.value.sequence >= MAX_MUTATION_RECOVERIES ||
      !Number.isSafeInteger(claim.value.slot_sequence) ||
      claim.value.slot_sequence < 0 ||
      claim.value.slot_sequence >= MAX_MUTATION_SLOTS ||
      claim.value.fixture_id !== fixtureId ||
      claim.value.action !== slot.value.action ||
      !onlyLowerHex(claim.value.lease_sha256, 64) ||
      claim.value.lease_sha256 !== (leaseSha256 ?? claims[0].value.lease_sha256) ||
      claim.value.chain_root_sha256 !==
        recoveryChainRootSha256(claim.value.fixture_id, claim.value.lease_sha256, slot.value) ||
      !onlyLowerHex(claim.value.nonce, 32) ||
      claim.value.operation_kind !== slot.value.operation_kind ||
      claim.value.operation_contract_sha256 !== slot.value.operation_contract_sha256 ||
      claim.value.operation_plan_sha256 !== operationPlanSha256(slot.value.operation_plan) ||
      !new Set(["complete", "pending", "not-reached"]).has(
        claim.value.observed_effect_disposition,
      ) ||
      typeof claim.value.observed_effect_name !== "string" ||
      !/^[a-z][a-z0-9-]{0,63}$/.test(claim.value.observed_effect_name) ||
      !onlyLowerHex(claim.value.observed_evidence_head_sha256, 64) ||
      !onlyLowerHex(claim.value.observed_evidence_prefix_sha256, 64) ||
      typeof claim.value.observed_evidence_stage !== "string" ||
      !/^[a-z][a-z0-9-]{0,63}$/.test(claim.value.observed_evidence_stage) ||
      !onlyLowerHex(claim.value.observed_residual_sha256, 64) ||
      !onlyLowerHex(claim.value.observed_settlement_sha256, 64) ||
      !onlyLowerHex(claim.value.parent_sha256, 64) ||
      !onlyLowerHex(claim.value.source_head_sha256, 64)
    ) {
      fail("mutation recovery claim was refused");
    }
    if (
      (slot.value.operation_kind === DETERMINISTIC_PROVIDER_OPERATION_KIND ||
        slot.value.action === LIVE_PROVIDER_PLAN_ACTION ||
        slot.value.action === COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
        slot.value.action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION) &&
      canonical({
        observed_effect_disposition: claim.value.observed_effect_disposition,
        observed_effect_name: claim.value.observed_effect_name,
        observed_evidence_head_sha256: claim.value.observed_evidence_head_sha256,
        observed_evidence_prefix_sha256: claim.value.observed_evidence_prefix_sha256,
        observed_evidence_stage: claim.value.observed_evidence_stage,
        observed_residual_sha256: claim.value.observed_residual_sha256,
        observed_settlement_sha256: claim.value.observed_settlement_sha256,
      }) !== canonical({
        observed_effect_disposition: "not-reached",
        observed_effect_name: "none",
        observed_evidence_head_sha256: ZERO_SHA256,
        observed_evidence_prefix_sha256: ZERO_SHA256,
        observed_evidence_stage: "none",
        observed_residual_sha256: ZERO_SHA256,
        observed_settlement_sha256: ZERO_SHA256,
      })
    ) {
      fail("deterministic provider recovery observation was refused");
    }
    if (slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION) {
      const stage =
        PROVIDER_RESERVATION_RECOVERY_STAGES[
          claim.value.observed_evidence_stage
        ];
      const evidenceExpected =
        claim.value.observed_evidence_stage !== "not-started";
      const expectedSettlementSha256 =
        operation === undefined ? ZERO_SHA256 : digest(operation.bytes);
      const topologySha256 =
        stage === undefined || !evidenceExpected
          ? ZERO_SHA256
          : reservationRecoveryTopologySha256({
              evidenceSha256:
                claim.value.observed_evidence_head_sha256,
              evidenceStage: claim.value.observed_evidence_stage,
              localLinks: stage.links,
              settlementSha256:
                claim.value.observed_settlement_sha256,
              slotSequence: claim.value.slot_sequence,
            });
      if (
        stage === undefined ||
        claim.value.observed_effect_name !== "provider-reservation" ||
        claim.value.observed_effect_disposition !== stage.disposition ||
        (!evidenceExpected &&
          (claim.value.observed_evidence_head_sha256 !== ZERO_SHA256 ||
            claim.value.observed_evidence_prefix_sha256 !== ZERO_SHA256 ||
            claim.value.observed_residual_sha256 !== ZERO_SHA256)) ||
        (evidenceExpected &&
          (claim.value.observed_evidence_head_sha256 === ZERO_SHA256 ||
            claim.value.observed_evidence_prefix_sha256 !== topologySha256 ||
            claim.value.observed_residual_sha256 !== topologySha256)) ||
        stage.settlement !==
          (claim.value.observed_settlement_sha256 !== ZERO_SHA256) ||
        (claim.value.observed_settlement_sha256 !== ZERO_SHA256 &&
          claim.value.observed_settlement_sha256 !==
            expectedSettlementSha256)
      ) {
        fail("live provider reservation recovery observation was refused");
      }
      if (
        previous === undefined &&
        new Set([
          "marker-reacquired",
          "stage-retired-before-effect",
        ]).has(claim.value.observed_evidence_stage)
      ) {
        fail("live provider reservation recovery history was refused");
      }
      if (
        previous?.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION
      ) {
        const previousStage =
          previous.value.observed_evidence_stage;
        if (
          !liveProviderReservationRecoveryStageReachable(
            previousStage,
            claim.value.observed_evidence_stage,
          ) ||
          (previousStage !== "not-started" &&
            previous.value.observed_evidence_head_sha256 !==
              claim.value.observed_evidence_head_sha256) ||
          (PROVIDER_RESERVATION_RECOVERY_STAGES[previousStage].settlement &&
            previous.value.observed_settlement_sha256 !==
              claim.value.observed_settlement_sha256)
        ) {
          fail("live provider reservation recovery history was refused");
        }
      }
    }
    if (slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION) {
      const stage =
        PROVIDER_EFFECT_RECOVERY_STAGES[
          claim.value.observed_evidence_stage
        ];
      if (
        stage === undefined ||
        claim.value.observed_effect_name !== "provider-effect" ||
        claim.value.observed_effect_disposition !== stage.disposition ||
        claim.value.observed_settlement_sha256 !== ZERO_SHA256 ||
        (claim.value.observed_evidence_stage === "not-started"
          ? claim.value.observed_evidence_head_sha256 !== ZERO_SHA256 ||
            claim.value.observed_evidence_prefix_sha256 !== ZERO_SHA256 ||
            claim.value.observed_residual_sha256 !== ZERO_SHA256
          : claim.value.observed_evidence_prefix_sha256 === ZERO_SHA256 ||
            claim.value.observed_residual_sha256 !==
              claim.value.observed_evidence_prefix_sha256) ||
        (previous?.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
          !liveProviderEffectRecoveryStageReachable(
            previous.value.observed_evidence_stage,
            claim.value.observed_evidence_stage,
          ))
      ) {
        fail("live provider effect recovery observation was refused");
      }
    }
    if (slot.value.operation_kind === CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND) {
      const cleanupStages = new Set([
        "not-started",
        "plan-publication-pending",
        "retiring",
        "progress-publication-pending",
        "progress-complete",
        "settlement-publication-pending",
        "settled",
      ]);
      const expectedDisposition =
        claim.value.observed_evidence_stage === "not-started"
          ? "not-reached"
          : claim.value.observed_evidence_stage === "settled"
            ? "complete"
            : "pending";
      if (
        slot.value.action !== "provider-cleanup" ||
        !cleanupStages.has(claim.value.observed_evidence_stage) ||
        claim.value.observed_effect_name !== "provider-cleanup" ||
        claim.value.observed_effect_disposition !== expectedDisposition ||
        claim.value.observed_evidence_head_sha256 !==
          slot.value.operation_plan.provider_identity_sha256 ||
        claim.value.observed_evidence_prefix_sha256 === ZERO_SHA256 ||
        claim.value.observed_residual_sha256 === ZERO_SHA256
      ) {
        fail("background cleanup recovery observation was refused");
      }
      const observed = claimObservation(claim);
      if (
        observed.observed_settlement_sha256 !== ZERO_SHA256 &&
        observed.observed_evidence_stage !== "settled"
      ) {
        fail("background cleanup recovery settlement observation was refused");
      }
      if (
        previous !== undefined &&
        previous.value.operation_kind ===
          CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
      ) {
        const previousObserved = claimObservation(previous);
        const previousSettledPrefix =
          previousObserved.observed_evidence_stage === "settled";
        const previousOuterSettlement =
          previousObserved.observed_settlement_sha256 !== ZERO_SHA256;
        const stageRank = new Map([
          ["not-started", 0],
          ["plan-publication-pending", 1],
          ["retiring", 2],
          ["progress-publication-pending", 3],
          ["progress-complete", 4],
          ["settlement-publication-pending", 5],
          ["settled", 6],
        ]);
        const previousStage = previousObserved.observed_evidence_stage;
        const currentStage = observed.observed_evidence_stage;
        const publicationRollback = new Set([
          "plan-publication-pending:not-started",
          "progress-publication-pending:retiring",
          "settlement-publication-pending:progress-complete",
        ]).has(`${previousStage}:${currentStage}`);
        if (
          (stageRank.get(currentStage) < stageRank.get(previousStage) &&
            !publicationRollback) ||
          (previousSettledPrefix &&
            (observed.observed_evidence_stage !== "settled" ||
              canonical({
                ...observed,
                observed_settlement_sha256: ZERO_SHA256,
              }) !==
                canonical({
                  ...previousObserved,
                  observed_settlement_sha256: ZERO_SHA256,
                }))) ||
          (previousOuterSettlement &&
            canonical(observed) !== canonical(previousObserved))
        ) {
          fail("background cleanup recovery settlement history was refused");
        }
      }
    }
    if (
      (previous === undefined &&
        (claim.value.sequence !== 0 || claim.value.parent_sha256 !== ZERO_SHA256)) ||
      (previous !== undefined &&
        (claim.value.sequence !== previous.value.sequence + 1 ||
          claim.value.parent_sha256 !== digest(previous.bytes)))
    ) {
      fail("mutation recovery claim chain was refused");
    }
    validateMutationOwner(claim.value, "owner", "mutation recovery claim");
    previous = claim;
  }
}

function authoritativeRecoveriesAtClose(recoveries, close, slot) {
  if (close.value.authority === "owner") {
    return close.value.authority_sha256 === digest(slot.bytes) &&
      recoveries.length === 0
      ? []
      : undefined;
  }
  const authorityIndex = recoveries.findIndex(
    (recovery) => digest(recovery.bytes) === close.value.authority_sha256,
  );
  return authorityIndex !== -1 && authorityIndex === recoveries.length - 1
    ? recoveries
    : undefined;
}

function validateMutationCloseValue(value, slot, fixtureId) {
  exactKeys(
    value,
    [
      "authority",
      "authority_sha256",
      "disposition",
      "fixture_id",
      "operation_evidence_sha256",
      "operation_contract_sha256",
      "operation_kind",
      "operation_plan_sha256",
      "result_head_sha256",
      "result_environment_sha256",
      "result_sequence",
      "schema",
      "slot_sequence",
      "slot_sha256",
    ],
    "mutation close",
  );
  if (
    value.schema !== MUTATION_CLOSE_SCHEMA ||
    value.fixture_id !== fixtureId ||
    value.slot_sequence !== slot.value.journal_sequence ||
    value.slot_sha256 !== digest(slot.bytes) ||
    value.operation_kind !== slot.value.operation_kind ||
    value.operation_contract_sha256 !== slot.value.operation_contract_sha256 ||
    value.operation_plan_sha256 !== operationPlanSha256(slot.value.operation_plan) ||
    !new Set(["owner", "recovery"]).has(value.authority) ||
    !onlyLowerHex(value.authority_sha256, 64) ||
    !onlyLowerHex(value.operation_evidence_sha256, 64) ||
    !onlyLowerHex(value.result_environment_sha256, 64) ||
    !new Set(["aborted-before-effect", "completed"]).has(value.disposition) ||
    !Number.isSafeInteger(value.result_sequence) ||
    value.result_sequence < slot.value.source_sequence ||
    value.result_sequence > 63 ||
    !onlyLowerHex(value.result_head_sha256, 64)
  ) {
    fail("mutation close was refused");
  }
}

function validateBackgroundCreateSettlement(value, slot, fixtureId) {
  exactKeys(
    value,
    [
      "authority",
      "authority_sha256",
      "controller_presence",
      "disposition",
      "effect_disposition",
      "effect_name",
      "evidence_head_sha256",
      "evidence_prefix_sha256",
      "evidence_stage",
      "fixture_id",
      "hostagent_presence",
      "operation_contract_sha256",
      "operation_kind",
      "operation_plan_sha256",
      "pending_evidence_publication",
      "pending_private_publications",
      "residual_sha256",
      "root_disposition",
      "safe_code",
      "schema",
      "slot_sequence",
      "slot_sha256",
      "sockets",
      "source_head_sha256",
      "static_root_identity_sha256",
    ],
    "mutation operation settlement",
  );
  validatePendingEvidenceSnapshot(value.pending_evidence_publication);
  validatePendingPrivateSnapshots(value.pending_private_publications);
  if (
    value.schema !== BACKGROUND_CREATE_SETTLEMENT_SCHEMA ||
    value.fixture_id !== fixtureId ||
    slot.value.action !== "provider-create" ||
    slot.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
    value.slot_sequence !== slot.value.journal_sequence ||
    value.slot_sha256 !== digest(slot.bytes) ||
    value.operation_kind !== slot.value.operation_kind ||
    value.operation_contract_sha256 !== slot.value.operation_contract_sha256 ||
    value.operation_plan_sha256 !== operationPlanSha256(slot.value.operation_plan) ||
    !new Set(["owner", "recovery"]).has(value.authority) ||
    !onlyLowerHex(value.authority_sha256, 64) ||
    !new Set(["not-started", "observed-present", "proved-absent", "unattested"]).has(
      value.controller_presence,
    ) ||
    !new Set(["complete-identity", "exact-residual"]).has(value.disposition) ||
    !new Set(["complete", "pending"]).has(value.effect_disposition) ||
    typeof value.effect_name !== "string" ||
    !/^[a-z][a-z0-9-]{0,63}$/.test(value.effect_name) ||
    !onlyLowerHex(value.evidence_head_sha256, 64) ||
    !onlyLowerHex(value.evidence_prefix_sha256, 64) ||
    typeof value.evidence_stage !== "string" ||
    !/^[a-z][a-z0-9-]{0,63}$/.test(value.evidence_stage) ||
    !new Set(["not-started", "observed-present", "proved-absent", "unattested"]).has(
      value.hostagent_presence,
    ) ||
    !onlyLowerHex(value.residual_sha256, 64) ||
    !new Set(["absent", "owned", "ownership-pending"]).has(value.root_disposition) ||
    !new Set(["evidence-refused", "none", "resource-collision"]).has(value.safe_code) ||
    !new Set(["absent", "partial", "present", "uninspected"]).has(value.sockets) ||
    !onlyLowerHex(value.source_head_sha256, 64) ||
    !onlyLowerHex(value.static_root_identity_sha256, 64) ||
    (value.disposition === "complete-identity" &&
      (value.effect_name !== "provider-identity" ||
        value.effect_disposition !== "complete" ||
        value.evidence_stage !== "provider-identity" ||
        value.safe_code !== "none" ||
        value.root_disposition !== "owned" ||
        value.controller_presence !== "proved-absent" ||
        !new Set(["observed-present", "proved-absent"]).has(value.hostagent_presence) ||
        value.pending_private_publications.length !== 0 ||
        (value.pending_evidence_publication !== null &&
          (value.pending_evidence_publication.target_name !== "provider-identity.json" ||
            value.pending_evidence_publication.disposition !== "linked-complete")) ||
        !new Set(["absent", "partial", "present"]).has(value.sockets))) ||
    (value.disposition === "exact-residual" &&
      (value.safe_code === "none" ||
        settleableBackgroundCompleteIdentity(value) ||
        (value.safe_code === "resource-collision" &&
          !settleableBackgroundResourceCollision(value)) ||
        (value.safe_code === "evidence-refused" &&
          (!new Set(["absent", "owned"]).has(value.root_disposition) ||
            !new Set(["not-started", "proved-absent"]).has(value.controller_presence) ||
            !new Set(["not-started", "proved-absent"]).has(value.hostagent_presence) ||
            !new Set(["absent", "partial", "present"]).has(value.sockets)))))
  ) {
    fail("mutation operation settlement was refused");
  }
}

function validateBackgroundCleanupSettlement(value, slot, fixtureId) {
  exactKeys(
    value,
    [
      "authority",
      "authority_sha256",
      "cleanup_plan_sha256",
      "disposition",
      "effect_disposition",
      "effect_name",
      "evidence_head_sha256",
      "evidence_prefix_sha256",
      "evidence_stage",
      "fixture_id",
      "operation_contract_sha256",
      "operation_kind",
      "operation_plan_sha256",
      "provider_retirement_settlement_sha256",
      "residual_sha256",
      "result_receipt_authorized",
      "root_disposition",
      "safe_code",
      "schema",
      "slot_sequence",
      "slot_sha256",
      "source_head_sha256",
    ],
    "background cleanup settlement",
  );
  if (
    value.schema !== BACKGROUND_CLEANUP_SETTLEMENT_SCHEMA ||
    value.fixture_id !== fixtureId ||
    slot.value.action !== "provider-cleanup" ||
    slot.value.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
    value.slot_sequence !== slot.value.journal_sequence ||
    value.slot_sha256 !== digest(slot.bytes) ||
    value.operation_kind !== slot.value.operation_kind ||
    value.operation_contract_sha256 !== slot.value.operation_contract_sha256 ||
    value.operation_plan_sha256 !== operationPlanSha256(slot.value.operation_plan) ||
    !new Set(["owner", "recovery"]).has(value.authority) ||
    !onlyLowerHex(value.authority_sha256, 64) ||
    !onlyLowerHex(value.cleanup_plan_sha256, 64) ||
    value.cleanup_plan_sha256 === ZERO_SHA256 ||
    value.disposition !== "complete-retirement" ||
    value.effect_disposition !== "complete" ||
    value.effect_name !== "provider-cleanup" ||
    value.evidence_head_sha256 !==
      slot.value.operation_plan.provider_identity_sha256 ||
    !onlyLowerHex(value.evidence_prefix_sha256, 64) ||
    value.evidence_prefix_sha256 === ZERO_SHA256 ||
    value.evidence_stage !== "settled" ||
    !onlyLowerHex(value.provider_retirement_settlement_sha256, 64) ||
    value.provider_retirement_settlement_sha256 === ZERO_SHA256 ||
    !onlyLowerHex(value.residual_sha256, 64) ||
    value.residual_sha256 === ZERO_SHA256 ||
    value.result_receipt_authorized !== true ||
    value.root_disposition !== "retired" ||
    value.safe_code !== "none" ||
    value.source_head_sha256 !== slot.value.intent_receipt_sha256
  ) {
    fail("background cleanup settlement was refused");
  }
}

function validateMutationOperationSettlement(value, slot, fixtureId) {
  if (slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION) {
    // The reservation source and durable witness are reconstructed only after
    // the complete live-decision journal has been validated below.
    return;
  }
  if (
    slot.value.action === "provider-create" &&
    slot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND
  ) {
    return validateBackgroundCreateSettlement(value, slot, fixtureId);
  }
  if (
    slot.value.action === "provider-cleanup" &&
    slot.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
  ) {
    return validateBackgroundCleanupSettlement(value, slot, fixtureId);
  }
  fail("mutation operation settlement action was refused");
}

function validatePendingEvidenceSnapshot(value) {
  if (value === null) return;
  exactKeys(
    value,
    [
      "actual_sha256",
      "declared_sha256",
      "device",
      "disposition",
      "inode",
      "links",
      "mode",
      "name",
      "size",
      "target_name",
      "uid",
    ],
    "mutation operation evidence publication",
  );
  if (
    !onlyLowerHex(value.actual_sha256, 64) ||
    !onlyLowerHex(value.declared_sha256, 64) ||
    !new Set(["linked-complete", "staged-complete", "staged-partial"]).has(
      value.disposition,
    ) ||
    !new Set([1, 2]).has(value.links) ||
    !new Set(["0600"]).has(value.mode) ||
    !/^[0-9]+$/.test(value.device) ||
    !/^[0-9]+$/.test(value.inode) ||
    !/^[0-9]+$/.test(value.size) ||
    !/^[0-9]+$/.test(value.uid) ||
    !/^\.provider-process-stage-[a-z0-9-]+-[0-9a-f]{64}-[0-9a-f]{32}$/.test(value.name) ||
    !/^[a-z][a-z0-9-]*\.json$/.test(value.target_name)
  ) {
    fail("mutation operation evidence publication was refused");
  }
}

function validatePendingPrivateSnapshots(value) {
  if (!Array.isArray(value) || value.length > 1) {
    fail("mutation operation private publications were refused");
  }
  for (const publication of value) {
    exactKeys(
      publication,
      [
        "actual_sha256",
        "declared_sha256",
        "device",
        "disposition",
        "inode",
        "links",
        "mode",
        "name",
        "size",
        "target_path",
        "uid",
      ],
      "mutation operation private publication",
    );
    if (
      !onlyLowerHex(publication.actual_sha256, 64) ||
      !onlyLowerHex(publication.declared_sha256, 64) ||
      !new Set(["linked-complete", "staged-complete", "staged-partial"]).has(
        publication.disposition,
      ) ||
      !new Set([1, 2]).has(publication.links) ||
      publication.mode !== "0600" ||
      !/^[0-9]+$/.test(publication.device) ||
      !/^[0-9]+$/.test(publication.inode) ||
      !/^[0-9]+$/.test(publication.size) ||
      !/^[0-9]+$/.test(publication.uid) ||
      !/^\.background-private-stage-[0-9a-f]{64}-[0-9a-f]{64}-[0-9a-f]{32}$/.test(
        publication.name,
      ) ||
      typeof publication.target_path !== "string" ||
      publication.target_path.length < 1
    ) {
      fail("mutation operation private publication was refused");
    }
  }
}

function backgroundPrefixObservation(prefix) {
  return Object.freeze({
    observed_effect_disposition: prefix.effectFrontier.disposition,
    observed_effect_name: prefix.effectFrontier.effect,
    observed_evidence_head_sha256: prefix.evidenceHeadSha256,
    observed_evidence_prefix_sha256: prefix.evidencePrefixSha256,
    observed_evidence_stage: prefix.evidenceStage,
    observed_residual_sha256: prefix.residualSha256,
    observed_settlement_sha256: ZERO_SHA256,
  });
}

function settlementObservation(settlement) {
  return Object.freeze({
    observed_effect_disposition: settlement.value.effect_disposition,
    observed_effect_name: settlement.value.effect_name,
    observed_evidence_head_sha256: settlement.value.evidence_head_sha256,
    observed_evidence_prefix_sha256: settlement.value.evidence_prefix_sha256,
    observed_evidence_stage: settlement.value.evidence_stage,
    observed_residual_sha256: settlement.value.residual_sha256,
    observed_settlement_sha256: digest(settlement.bytes),
  });
}

function claimObservation(claim) {
  return Object.freeze({
    observed_effect_disposition: claim.value.observed_effect_disposition,
    observed_effect_name: claim.value.observed_effect_name,
    observed_evidence_head_sha256: claim.value.observed_evidence_head_sha256,
    observed_evidence_prefix_sha256: claim.value.observed_evidence_prefix_sha256,
    observed_evidence_stage: claim.value.observed_evidence_stage,
    observed_residual_sha256: claim.value.observed_residual_sha256,
    observed_settlement_sha256: claim.value.observed_settlement_sha256,
  });
}

function settlementSnapshot(prefix) {
  return Object.freeze({
    controller_presence: prefix.residual.controller_presence,
    effect_disposition: prefix.effectFrontier.disposition,
    effect_name: prefix.effectFrontier.effect,
    evidence_head_sha256: prefix.evidenceHeadSha256,
    evidence_prefix_sha256: prefix.evidencePrefixSha256,
    evidence_stage: prefix.evidenceStage,
    hostagent_presence: prefix.residual.hostagent_presence,
    pending_evidence_publication: prefix.pendingPublication ?? null,
    pending_private_publications: prefix.residual.private_publications,
    residual_sha256: prefix.residualSha256,
    root_disposition: prefix.residual.root_disposition,
    sockets: prefix.residual.sockets,
    static_root_identity_sha256: prefix.residual.static_root_identity_sha256,
  });
}

function settleableBackgroundCompleteIdentity(snapshot) {
  return (
    snapshot.effect_disposition === "complete" &&
    snapshot.effect_name === "provider-identity" &&
    snapshot.evidence_stage === "provider-identity" &&
    snapshot.root_disposition === "owned" &&
    snapshot.controller_presence === "proved-absent" &&
    new Set(["observed-present", "proved-absent"]).has(
      snapshot.hostagent_presence,
    ) &&
    new Set(["absent", "partial", "present"]).has(snapshot.sockets) &&
    snapshot.pending_private_publications.length === 0 &&
    (snapshot.pending_evidence_publication === null ||
      (snapshot.pending_evidence_publication.target_name === "provider-identity.json" &&
        snapshot.pending_evidence_publication.disposition === "linked-complete"))
  );
}

function settleableBackgroundResourceCollision(snapshot) {
  return (
    snapshot.effect_disposition === "complete" &&
    snapshot.effect_name === "provider-root-collision" &&
    new Set(["empty", "create-authority"]).has(snapshot.evidence_stage) &&
    snapshot.root_disposition === "ownership-pending" &&
    snapshot.controller_presence === "not-started" &&
    snapshot.hostagent_presence === "not-started" &&
    snapshot.sockets === "uninspected" &&
    snapshot.pending_private_publications.length === 0 &&
    (snapshot.pending_evidence_publication === null ||
      snapshot.pending_evidence_publication.target_name ===
        "background-create-authority.json")
  );
}

function preIntentBackgroundResourceCollision(prefix) {
  const snapshot = settlementSnapshot(prefix);
  return (
    settleableBackgroundResourceCollision(snapshot) &&
    snapshot.evidence_stage === "empty" &&
    snapshot.pending_evidence_publication === null
  );
}

function backgroundSettlementDecision(prefix) {
  const snapshot = settlementSnapshot(prefix);
  const completeIdentity = settleableBackgroundCompleteIdentity(snapshot);
  if (completeIdentity) {
    return Object.freeze({ disposition: "complete-identity", safeCode: "none" });
  }
  const collision = settleableBackgroundResourceCollision(snapshot);
  if (collision) {
    return Object.freeze({ disposition: "exact-residual", safeCode: "resource-collision" });
  }
  const exactResidual =
    new Set(["complete", "pending"]).has(snapshot.effect_disposition) &&
    new Set(["absent", "owned"]).has(snapshot.root_disposition) &&
    new Set(["not-started", "proved-absent"]).has(snapshot.controller_presence) &&
    new Set(["not-started", "proved-absent"]).has(snapshot.hostagent_presence) &&
    new Set(["absent", "partial", "present"]).has(snapshot.sockets);
  if (exactResidual) {
    return Object.freeze({ disposition: "exact-residual", safeCode: "evidence-refused" });
  }
  return undefined;
}

function validateCandidate(candidate) {
  exactKeys(
    candidate,
    [
      "created_at",
      "excluded_claims",
      "feature",
      "fixtures",
      "kind",
      "requested_assertions",
      "run_id",
      "schema_version",
      "selection",
      "source",
    ],
    "candidate",
  );
  if (
    candidate.kind !== "synveda-cpr45-clean-engine-candidate" ||
    candidate.schema_version !== 1 ||
    candidate.feature !== "CPR-45" ||
    !onlyLowerHex(candidate.run_id, 32) ||
    typeof candidate.created_at !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(candidate.created_at) ||
    Number.isNaN(Date.parse(candidate.created_at)) ||
    JSON.stringify(candidate.requested_assertions) !== JSON.stringify(REQUESTED_ASSERTIONS) ||
    JSON.stringify(candidate.excluded_claims) !== JSON.stringify(EXCLUDED_CLAIMS)
  ) {
    fail("candidate contract was refused");
  }
  exactKeys(
    candidate.source,
    [
      "build_context_manifest_sha256",
      "commit_sha",
      "deployment_contract_sha256",
      "deployment_input_manifest_sha256",
      "tracked_index_manifest_sha256",
      "tree_sha",
      "worktree_clean",
    ],
    "candidate source",
  );
  for (const name of [
    "build_context_manifest_sha256",
    "deployment_contract_sha256",
    "deployment_input_manifest_sha256",
    "tracked_index_manifest_sha256",
  ]) {
    if (!onlyLowerHex(candidate.source[name], 64)) fail("candidate source digest was refused");
  }
  if (
    !onlyLowerHex(candidate.source.commit_sha, 40) ||
    !onlyLowerHex(candidate.source.tree_sha, 40) ||
    candidate.source.worktree_clean !== true
  ) {
    fail("candidate source identity was refused");
  }
  exactKeys(
    candidate.selection,
    [
      "app_host",
      "auth_host",
      "ipv4_pool",
      "oidc",
      "port",
      "postgres",
      "profiles",
      "project",
      "project_suffix",
      "runtime",
      "scheme",
    ],
    "candidate selection",
  );
  const suffix = `acceptance-${candidate.run_id.slice(0, 24)}`;
  if (
    candidate.selection.runtime !== "development" ||
    candidate.selection.postgres !== "bundled" ||
    candidate.selection.oidc !== "bundled" ||
    candidate.selection.project_suffix !== suffix ||
    candidate.selection.project !== `synveda-development-${suffix}` ||
    JSON.stringify(candidate.selection.profiles) !==
      JSON.stringify(["browser-acceptance", "demo"]) ||
    !privateIpv4Pool(candidate.selection.ipv4_pool) ||
    candidate.selection.app_host !== "app.synveda.test" ||
    candidate.selection.auth_host !== "auth.synveda.test" ||
    candidate.selection.scheme !== "http" ||
    candidate.selection.port !== 8080
  ) {
    fail("candidate selection was refused");
  }
  exactKeys(
    candidate.fixtures,
    [
      "builder_canary",
      "docker_proxy",
      "registry_authentication",
      "registry_image",
      "registry_transport",
    ],
    "candidate fixtures",
  );
  if (
    candidate.fixtures.registry_image !== REGISTRY_IMAGE ||
    candidate.fixtures.registry_transport !== "loopback-tls-ephemeral" ||
    candidate.fixtures.registry_authentication !== "one-run-basic-bcrypt" ||
    candidate.fixtures.docker_proxy !== "synthetic-nonsecret-v1" ||
    candidate.fixtures.builder_canary !== "ambient-remote-inert-zero-read-v1"
  ) {
    fail("candidate fixtures were refused");
  }
}

function validatePlan(plan, candidateBytes, stateMetadata) {
  exactKeys(
    plan,
    ["fixture_id", "outcome", "phase", "previous_sha256", "result", "schema", "sequence"],
    "plan receipt",
  );
  if (
    plan.schema !== "synveda.clean-engine.receipt.v6" ||
    plan.sequence !== 0 ||
    plan.outcome !== "passed" ||
    plan.phase !== "plan" ||
    plan.previous_sha256 !== ZERO_SHA256 ||
    !onlyLowerHex(plan.fixture_id, 32)
  ) {
    fail("plan receipt contract was refused");
  }
  exactKeys(
    plan.result,
    [
      "candidate_sha256",
      "project",
      "provider",
      "provider_resource",
      "state_device",
      "state_inode",
    ],
    "plan result",
  );
  if (
    plan.result.candidate_sha256 !== digest(candidateBytes) ||
    plan.result.provider !== "colima" ||
    plan.result.project !== `synveda-development-acceptance-${plan.fixture_id.slice(0, 24)}` ||
    plan.result.provider_resource !== `synveda-cpr45-${plan.fixture_id}` ||
    plan.result.state_device !== String(stateMetadata.dev) ||
    plan.result.state_inode !== String(stateMetadata.ino)
  ) {
    fail("plan result was refused");
  }
}

function validateProxyTemplate(path) {
  const { value } = parseCanonical(path, "proxy template");
  if (canonical(value) !== canonical(PROXY_TEMPLATE)) fail("proxy template contract was refused");
}

function sameStringEntries(left, right) {
  return (
    left.length === right.length &&
    left.every((entry, index) => entry === right[index])
  );
}

function sameMutationStageIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.uid === right.uid &&
    left.mode === right.mode &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs
  );
}

function isMutationPublicationDestinationName(name) {
  return (
    /^\.mutation-(?:slot|close|operation)-[0-9]{2}$/.test(name) ||
    /^\.mutation-recovery-[0-9]{2}-[0-9]{2}$/.test(name) ||
    /^\.provider-effect-event-[0-9]{2}-[0-9]{3}$/.test(name)
  );
}

// Mutation publishers retire private aliases cooperatively. Validate only a
// directory generation that stayed exact for the whole scan; a missing alias
// must never be skipped while the rest of its stale inventory is accepted.
function planRunInventorySnapshotTransition(left, right) {
  if (
    !left.metadata.isDirectory() ||
    !right.metadata.isDirectory() ||
    left.metadata.dev !== right.metadata.dev ||
    left.metadata.ino !== right.metadata.ino ||
    left.metadata.uid !== right.metadata.uid ||
    left.metadata.mode !== right.metadata.mode
  ) {
    return "refused";
  }
  const commonStageNames = [...left.mutationStages.keys()].filter((name) =>
    right.mutationStages.has(name),
  );
  let mutationStageMetadataChanged = false;
  for (const name of commonStageNames) {
    const metadata = left.mutationStages.get(name);
    const current = right.mutationStages.get(name);
    if (!sameMutationStageIdentity(metadata, current)) {
      return "refused";
    }
    mutationStageMetadataChanged ||= !sameMetadata(metadata, current);
  }
  const leftEntries = new Set(left.entries);
  const rightEntries = new Set(right.entries);
  for (const name of left.entries) {
    if (!rightEntries.has(name) && !/^\.mutation-stage-[0-9a-f]{32}$/.test(name)) {
      return "refused";
    }
  }
  for (const name of right.entries) {
    if (
      !leftEntries.has(name) &&
      !/^\.mutation-stage-[0-9a-f]{32}$/.test(name) &&
      !isMutationPublicationDestinationName(name)
    ) {
      return "refused";
    }
  }
  if (
    !sameStringEntries(left.entries, right.entries) ||
    mutationStageMetadataChanged
  ) {
    return "superseded";
  }
  return left.stable &&
    right.stable &&
    sameMetadata(left.metadata, right.metadata) &&
    left.mutationStages.size === right.mutationStages.size
    ? "same"
    : "superseded";
}

function capturePlanRunInventorySnapshot(run) {
  const before = ownedPrivateDirectory(run, "active run state");
  let entries;
  try {
    entries = readdirSync(run).sort();
  } catch {
    fail("plan run inventory was unavailable", 69);
  }
  const mutationStageNames = entries.filter((entry) =>
    /^\.mutation-stage-[0-9a-f]{32}$/.test(entry),
  );
  if (mutationStageNames.length > MAX_MUTATION_STAGES) {
    fail("plan run inventory was refused");
  }
  const mutationStages = new Map();
  let stageDisappeared = false;
  for (const name of mutationStageNames) {
    try {
      mutationStages.set(name, exactLstat(join(run, name)));
    } catch (error) {
      if (error?.code === "ENOENT") {
        stageDisappeared = true;
        break;
      }
      fail("pending mutation publication was unavailable", 69);
    }
  }
  const after = ownedPrivateDirectory(run, "active run state");
  return {
    entries,
    metadata: after,
    mutationStages,
    stageDisappeared,
    stable:
      !stageDisappeared &&
      before.uid === after.uid &&
      sameMetadata(before, after),
  };
}

function validatePlanRunInventorySnapshot(
  run,
  stateMetadata,
  entries,
  beforeMutationStageValidationObserver,
) {
  const required = [
    "00-plan.json",
    "candidate.json",
    "client",
    "evidence",
    "provider",
    "registry",
    "runtime",
  ];
  const receipts = entries.filter((entry) => /^[0-9]{2}-[a-z][a-z0-9-]*\.json$/.test(entry));
  const hasReceiptStaging = entries.includes(RECEIPT_STAGING_NAME);
  const hasEnvironment = entries.includes(ENVIRONMENT_NAME);
  const hasEnvironmentStaging = entries.includes(ENVIRONMENT_STAGING_NAME);
  const hasLegacyMutationLease = entries.includes(LEGACY_MUTATION_LEASE_NAME);
  const mutationSlotNames = entries.filter((entry) =>
    /^\.mutation-slot-[0-9]{2}$/.test(entry),
  );
  const mutationCloseNames = entries.filter((entry) =>
    /^\.mutation-close-[0-9]{2}$/.test(entry),
  );
  const mutationRecoveryNames = entries.filter((entry) =>
    /^\.mutation-recovery-[0-9]{2}-[0-9]{2}$/.test(entry),
  );
  const mutationOperationNames = entries.filter((entry) =>
    /^\.mutation-operation-[0-9]{2}$/.test(entry),
  );
  const mutationStageNames = entries.filter((entry) =>
    /^\.mutation-stage-[0-9a-f]{32}$/.test(entry),
  );
  const providerReservationStageNames = entries.filter((entry) =>
    /^\.provider-reservation-stage-[0-9a-f]{32}$/.test(entry),
  );
  const providerReservationWitnessNames = entries.filter((entry) =>
    /^\.provider-reservation-witness-[0-9]{2}$/.test(entry),
  );
  const providerEffectStageNames = entries.filter((entry) =>
    /^\.provider-effect-stage-[0-9]{2}-[0-9a-f]{32}$/.test(entry),
  );
  const providerEffectWitnessNames = entries.filter((entry) =>
    /^\.provider-effect-witness-[0-9]{2}$/.test(entry),
  );
  const providerEffectEventNames = entries.filter((entry) =>
    /^\.provider-effect-event-[0-9]{2}-[0-9]{3}$/.test(entry),
  );
  if (
    receipts.length < 1 ||
    receipts.length > 64 ||
    mutationSlotNames.length > MAX_MUTATION_SLOTS ||
    mutationCloseNames.length > MAX_MUTATION_SLOTS ||
    mutationRecoveryNames.length > MAX_MUTATION_SLOTS * MAX_MUTATION_RECOVERIES ||
    mutationOperationNames.length > MAX_MUTATION_SLOTS ||
    mutationStageNames.length > MAX_MUTATION_STAGES ||
    providerReservationStageNames.length > 1 ||
    providerReservationWitnessNames.length > 1 ||
    providerEffectStageNames.length > 1 ||
    providerEffectWitnessNames.length > 1 ||
    providerEffectEventNames.length >
      COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.effect_events ||
    required.some((entry) => !entries.includes(entry)) ||
    entries.some(
      (entry) =>
        !required.includes(entry) &&
        !receipts.includes(entry) &&
        entry !== RECEIPT_STAGING_NAME &&
        entry !== ENVIRONMENT_NAME &&
        entry !== ENVIRONMENT_STAGING_NAME &&
        entry !== LEGACY_MUTATION_LEASE_NAME &&
        !mutationSlotNames.includes(entry) &&
        !mutationCloseNames.includes(entry) &&
        !mutationRecoveryNames.includes(entry) &&
        !mutationOperationNames.includes(entry) &&
        !mutationStageNames.includes(entry) &&
        !providerReservationStageNames.includes(entry) &&
        !providerReservationWitnessNames.includes(entry) &&
        !providerEffectStageNames.includes(entry) &&
        !providerEffectWitnessNames.includes(entry) &&
        !providerEffectEventNames.includes(entry),
    )
  ) {
    fail("plan run inventory was refused");
  }
  if (hasLegacyMutationLease) {
    fail("legacy mutation lease was refused; prepare a fresh clean-engine plan");
  }
  let providerDirectory;
  for (const directory of ["client", "evidence", "provider", "registry", "runtime"]) {
    const path = join(run, directory);
    const metadata = ownedPrivateDirectory(path, "plan run directory");
    if (metadata.dev !== stateMetadata.dev) fail("plan run crossed a filesystem boundary");
    const children = readdirSync(path);
    if (directory === "client") {
      if (children.length !== 1 || children[0] !== "proxy-template.json") {
        fail("plan client inventory was refused");
      }
    } else if (directory === "provider") {
      providerDirectory = path;
    } else if (children.length !== 0) {
      fail("pre-provider plan directory was not empty");
    }
  }
  let pendingPublication;
  if (hasReceiptStaging) {
    const path = join(run, RECEIPT_STAGING_NAME);
    const metadata = inspectPendingFile(
      path,
      "pending receipt publication",
      stateMetadata.dev,
      new Set([1n, 2n]),
    );
    const linkedReceipts = receipts.filter((name) => {
      const candidate = exactLstat(join(run, name));
      return candidate.dev === metadata.dev && candidate.ino === metadata.ino;
    });
    if (
      (metadata.nlink === 1n && linkedReceipts.length !== 0) ||
      (metadata.nlink === 2n && linkedReceipts.length !== 1) ||
      linkedReceipts.includes("00-plan.json")
    ) {
      fail("pending receipt publication link was refused");
    }
    pendingPublication = {
      linkedReceipt: linkedReceipts[0],
      links: Number(metadata.nlink),
      path,
    };
  }
  let environmentPublication;
  if (hasEnvironmentStaging) {
    const path = join(run, ENVIRONMENT_STAGING_NAME);
    const metadata = inspectPendingFile(
      path,
      "pending environment publication",
      stateMetadata.dev,
      new Set([1n, 2n]),
    );
    let linkedEnvironment = false;
    if (hasEnvironment) {
      const candidate = exactLstat(join(run, ENVIRONMENT_NAME));
      linkedEnvironment = candidate.dev === metadata.dev && candidate.ino === metadata.ino;
    }
    if (
      (metadata.nlink === 1n && linkedEnvironment) ||
      (metadata.nlink === 2n && !linkedEnvironment)
    ) {
      fail("pending environment publication link was refused");
    }
    environmentPublication = {
      linkedEnvironment,
      links: Number(metadata.nlink),
      path,
    };
  }
  if (hasEnvironment) {
    const expectedLinks = environmentPublication?.linkedEnvironment ? new Set([2n]) : new Set([1n]);
    inspectPendingFile(
      join(run, ENVIRONMENT_NAME),
      "environment manifest",
      stateMetadata.dev,
      expectedLinks,
    );
  }
  // Resolve mutation staging links before parsing their final destinations.
  // A crash after link(2) but before the staging alias is retired leaves both
  // names at link count two; that is a recoverable publication boundary, not
  // permission for any unrelated hard link.
  const mutationDestinations = [
    ...mutationSlotNames,
    ...mutationCloseNames,
    ...mutationRecoveryNames,
    ...mutationOperationNames,
    ...providerEffectEventNames,
  ];
  const linkedMutationDestinations = new Map();
  const mutationStages = mutationStageNames.map((name) => {
    if (beforeMutationStageValidationObserver?.(name) !== undefined) {
      fail("mutation inventory test observer returned a value", 70);
    }
    const path = join(run, name);
    const metadata = inspectPendingFile(
      path,
      "pending mutation publication",
      stateMetadata.dev,
      new Set([1n, 2n]),
    );
    const linkedDestinations = mutationDestinations.filter((destination) => {
      if (!entries.includes(destination)) return false;
      const candidate = exactLstat(join(run, destination));
      return candidate.dev === metadata.dev && candidate.ino === metadata.ino;
    });
    if (
      (metadata.nlink === 1n && linkedDestinations.length !== 0) ||
      (metadata.nlink === 2n && linkedDestinations.length !== 1)
    ) {
      fail("pending mutation publication link was refused");
    }
    const linkedDestination = linkedDestinations[0];
    if (linkedDestination !== undefined) {
      if (linkedMutationDestinations.has(linkedDestination)) {
        fail("pending mutation publication link was refused");
      }
      linkedMutationDestinations.set(linkedDestination, name);
    }
    return { linkedDestination, metadata, name, path };
  });
  const mutationSlots = mutationSlotNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation slot",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  for (const [sequence, slot] of mutationSlots.entries()) {
    if (!onlyLowerHex(slot.value?.fixture_id, 32)) fail("mutation slot was refused");
    validateMutationLeaseValue(slot.value, slot.value.fixture_id);
    if (
      slot.name !== mutationSlotFileName(slot.value.journal_sequence) ||
      slot.value.journal_sequence !== sequence
    ) {
      fail("mutation slot sequence was refused");
    }
  }
  const readProviderLinkArtifact = (name, label) => {
    if (name === undefined) return undefined;
    const path = join(run, name);
    const metadata = inspectPendingFile(
      path,
      label,
      stateMetadata.dev,
      new Set([1n, 2n, 3n]),
    );
    return {
      ...parseCanonical(path, label, Number(metadata.nlink)),
      metadata,
      name,
      path,
    };
  };
  const providerReservationStage = readProviderLinkArtifact(
    providerReservationStageNames[0],
    "provider reservation stage",
  );
  const providerReservationWitness = readProviderLinkArtifact(
    providerReservationWitnessNames[0],
    "provider reservation witness",
  );
  if (
    providerReservationStage !== undefined &&
    providerReservationWitness !== undefined
  ) {
    if (
      !sameMutationArtifact(
        providerReservationStage.metadata,
        providerReservationWitness.metadata,
      ) ||
      providerReservationStage.metadata.nlink !== 3n ||
      providerReservationWitness.metadata.nlink !== 3n ||
      !providerReservationStage.bytes.equals(providerReservationWitness.bytes)
    ) {
      fail("provider reservation local link topology was refused");
    }
  } else {
    const onlyArtifact =
      providerReservationStage ?? providerReservationWitness;
    if (
      onlyArtifact !== undefined &&
      !new Set([1n, 2n]).has(onlyArtifact.metadata.nlink)
    ) {
      fail("provider reservation local link topology was refused");
    }
  }
  for (const artifact of [
    providerReservationStage,
    providerReservationWitness,
  ]) {
    if (artifact === undefined) continue;
    const sequence = artifact.value?.slot_sequence;
    const slot = Number.isSafeInteger(sequence)
      ? mutationSlots[sequence]
      : undefined;
    if (
      slot?.value.action !== COLIMA_LIVE_PROVIDER_RESERVATION_ACTION ||
      (artifact === providerReservationWitness &&
        artifact.name !== providerReservationWitnessFileName(sequence))
    ) {
      fail("provider reservation witness slot binding was refused");
    }
  }
  const readProviderEffectStage = (name) => {
    if (name === undefined) return undefined;
    const path = join(run, name);
    const metadata = inspectPendingFile(
      path,
      "provider effect stage",
      stateMetadata.dev,
      new Set([1n, 2n, 3n]),
    );
    const bytes = readPrivate(
      path,
      "provider effect stage",
      Number(metadata.nlink),
      0n,
    );
    let value;
    let canonicalValue = false;
    try {
      value = JSON.parse(bytes.toString("utf8"));
      canonicalValue = canonicalBytes(value).equals(bytes);
    } catch {
      canonicalValue = false;
    }
    const slotSequence = Number(name.slice(PROVIDER_EFFECT_STAGE_PREFIX.length, -33));
    if (!canonicalValue) {
      if (metadata.nlink !== 1n) {
        fail("linked provider effect stage was not canonical");
      }
      return {
        bytes,
        initializing: true,
        metadata,
        name,
        path,
        slotSequence,
        value: undefined,
      };
    }
    if (value === null || Array.isArray(value) || typeof value !== "object") {
      fail("provider effect stage was not a canonical object");
    }
    return {
      bytes,
      initializing: false,
      metadata,
      name,
      path,
      slotSequence,
      value,
    };
  };
  const providerEffectStage = readProviderEffectStage(
    providerEffectStageNames[0],
  );
  const providerEffectWitness = readProviderLinkArtifact(
    providerEffectWitnessNames[0],
    "provider effect witness",
  );
  if (
    providerEffectStage !== undefined &&
    providerEffectWitness !== undefined
  ) {
    if (
      providerEffectStage.initializing ||
      !sameMutationArtifact(
        providerEffectStage.metadata,
        providerEffectWitness.metadata,
      ) ||
      providerEffectStage.metadata.nlink !== 3n ||
      providerEffectWitness.metadata.nlink !== 3n ||
      !providerEffectStage.bytes.equals(providerEffectWitness.bytes)
    ) {
      fail("provider effect local link topology was refused");
    }
  } else {
    const onlyArtifact = providerEffectStage ?? providerEffectWitness;
    if (
      onlyArtifact !== undefined &&
      !new Set([1n, 2n]).has(onlyArtifact.metadata.nlink)
    ) {
      fail("provider effect local link topology was refused");
    }
  }
  for (const artifact of [providerEffectStage, providerEffectWitness]) {
    if (artifact === undefined) continue;
    const sequence =
      artifact === providerEffectStage
        ? artifact.slotSequence
        : artifact.value?.slot_sequence;
    const slot = Number.isSafeInteger(sequence)
      ? mutationSlots[sequence]
      : undefined;
    if (
      slot?.value.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
      (artifact === providerEffectStage &&
        artifact.name.slice(PROVIDER_EFFECT_STAGE_PREFIX.length, -33) !==
          String(sequence).padStart(2, "0")) ||
      (artifact === providerEffectStage &&
        !artifact.initializing && artifact.value.slot_sequence !== sequence) ||
      (artifact === providerEffectWitness &&
        artifact.name !== providerEffectWitnessFileName(sequence))
    ) {
      fail("provider effect witness slot binding was refused");
    }
  }
  const providerEffectEvents = providerEffectEventNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "provider effect event",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  const effectEventsBySlot = new Map();
  for (const event of providerEffectEvents) {
    const slotSequence = event.value?.slot_sequence;
    const eventSequence = event.value?.event_sequence;
    const slot = Number.isSafeInteger(slotSequence)
      ? mutationSlots[slotSequence]
      : undefined;
    const events = effectEventsBySlot.get(slotSequence) ?? [];
    if (
      slot?.value.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
      !Number.isSafeInteger(eventSequence) ||
      eventSequence !== events.length ||
      eventSequence >= COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.effect_events ||
      event.name !== providerEffectEventFileName(slotSequence, eventSequence)
    ) {
      fail("provider effect event sequence was refused");
    }
    events.push(event);
    effectEventsBySlot.set(slotSequence, events);
  }
  const mutationCloses = mutationCloseNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation close",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  const mutationOperations = mutationOperationNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation operation settlement",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  const operationsBySlot = new Map();
  for (const operation of mutationOperations) {
    const sequence = operation.value?.slot_sequence;
    const slot = Number.isSafeInteger(sequence) ? mutationSlots[sequence] : undefined;
    if (
      slot === undefined ||
      operation.name !== mutationOperationFileName(sequence) ||
      operationsBySlot.has(sequence)
    ) {
      fail("mutation operation settlement sequence was refused");
    }
    validateMutationOperationSettlement(operation.value, slot, slot.value.fixture_id);
    operationsBySlot.set(sequence, operation);
  }
  const closesBySlot = new Map();
  for (const close of mutationCloses) {
    const sequence = close.value?.slot_sequence;
    const slot = Number.isSafeInteger(sequence) ? mutationSlots[sequence] : undefined;
    if (
      slot === undefined ||
      close.name !== mutationCloseFileName(sequence) ||
      closesBySlot.has(sequence)
    ) {
      fail("mutation close sequence was refused");
    }
    validateMutationCloseValue(close.value, slot, slot.value.fixture_id);
    closesBySlot.set(sequence, close);
  }
  for (let sequence = 0; sequence < mutationSlots.length - 1; sequence += 1) {
    if (!closesBySlot.has(sequence)) {
      fail("mutation slot journal was not closed in order");
    }
  }
  const allMutationRecoveries = mutationRecoveryNames.map((name) => ({
    ...parseCanonical(
      join(run, name),
      "mutation recovery claim",
      linkedMutationDestinations.has(name) ? 2 : 1,
    ),
    name,
  }));
  const recoveriesBySlot = new Map();
  for (const recovery of allMutationRecoveries) {
    const sequence = recovery.value?.slot_sequence;
    if (!Number.isSafeInteger(sequence) || mutationSlots[sequence] === undefined) {
      fail("mutation recovery slot was refused");
    }
    const group = recoveriesBySlot.get(sequence) ?? [];
    group.push(recovery);
    recoveriesBySlot.set(sequence, group);
  }
  for (const [sequence, claims] of recoveriesBySlot) {
    const slot = mutationSlots[sequence];
    const recoverableAction =
      slot.value.action === "provider-create" ||
      slot.value.action === LIVE_PROVIDER_PLAN_ACTION ||
      slot.value.action === COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
      slot.value.action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
      slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION ||
      slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
      (slot.value.action === "provider-cleanup" &&
        slot.value.operation_kind ===
          CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND);
    if (!recoverableAction || claims.length > MAX_MUTATION_RECOVERIES) {
      fail("mutation recovery action was refused");
    }
    validateRecoveryClaims(
      claims,
      slot.value.fixture_id,
      slot,
      operationsBySlot.get(sequence),
    );
  }
  const lastSlot = mutationSlots.at(-1);
  const activeMutationSlot =
    lastSlot !== undefined && !closesBySlot.has(lastSlot.value.journal_sequence)
      ? lastSlot
      : undefined;
  const activeMutationRecoveries =
    activeMutationSlot === undefined
      ? []
      : recoveriesBySlot.get(activeMutationSlot.value.journal_sequence) ?? [];
  return {
    activeMutationRecoveries,
    activeMutationSlot,
    allMutationRecoveries,
    environment: hasEnvironment ? join(run, ENVIRONMENT_NAME) : undefined,
    environmentPublication,
    mutationCloses,
    mutationOperations,
    operationsBySlot,
    mutationLease: activeMutationSlot,
    mutationRecoveries: activeMutationRecoveries,
    mutationSlots,
    mutationStages,
    effectEventsBySlot,
    providerEffectEvents,
    providerEffectStage,
    providerEffectWitness,
    providerReservationStage,
    providerReservationWitness,
    pendingPublication,
    providerDirectory,
    receipts,
  };
}

function validatePlanRunInventory(
  run,
  beforeMutationStageValidationObserver,
) {
  if (
    beforeMutationStageValidationObserver !== undefined &&
    typeof beforeMutationStageValidationObserver !== "function"
  ) {
    fail("mutation inventory test observer was refused", 64);
  }
  for (
    let supersessions = 0;
    supersessions <= MAX_PLAN_RUN_INVENTORY_SUPERSESSIONS;
    supersessions += 1
  ) {
    const snapshot = capturePlanRunInventorySnapshot(run);
    if (!snapshot.stable) {
      const confirmed = capturePlanRunInventorySnapshot(run);
      if (
        planRunInventorySnapshotTransition(snapshot, confirmed) ===
        "superseded"
      ) {
        continue;
      }
      fail("plan run inventory changed outside mutation staging");
    }
    let inventory;
    let validationFailure;
    try {
      inventory = validatePlanRunInventorySnapshot(
        run,
        snapshot.metadata,
        snapshot.entries,
        beforeMutationStageValidationObserver,
      );
    } catch (error) {
      validationFailure = error;
    }
    const confirmed = capturePlanRunInventorySnapshot(run);
    const transition = planRunInventorySnapshotTransition(snapshot, confirmed);
    if (transition === "superseded") continue;
    if (transition === "refused") {
      fail("plan run inventory changed outside mutation staging");
    }
    if (validationFailure !== undefined) throw validationFailure;
    return { inventory, stateMetadata: snapshot.metadata };
  }
  fail("plan run inventory was repeatedly superseded", 73);
}

function providerEffectMarkerWitnessIdentitySha256(metadata, slotSha256) {
  return digest(canonicalBytes({
    device: String(metadata.dev),
    inode: String(metadata.ino),
    mode: modeString(metadata),
    schema: "synveda.clean-engine.provider-effect-physical-witness.v1",
    slot_sha256: slotSha256,
    uid: String(metadata.uid),
  }));
}

function providerEffectInitializingStageSha256(stage, slot) {
  if (
    stage?.initializing !== true ||
    slot?.value.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
    stage.slotSequence !== slot.value.journal_sequence
  ) {
    fail("initializing provider effect stage was refused", 70);
  }
  return digest(canonicalBytes({
    content_sha256: digest(stage.bytes),
    device: String(stage.metadata.dev),
    inode: String(stage.metadata.ino),
    mode: modeString(stage.metadata),
    name: stage.name,
    schema: "synveda.clean-engine.provider-effect-initializing-stage.v1",
    size: String(stage.metadata.size),
    slot_sequence: slot.value.journal_sequence,
    uid: String(stage.metadata.uid),
  }));
}

function validateProviderEffectStageEventConsistency(stage, events) {
  const kinds = events.map((event) => event.value.event_kind);
  const preEventStages = new Set([
    "marker-held",
    "marker-linked",
    "not-started",
    "stage-initializing",
    "stage-only",
    "stage-retired-before-effect",
    "witness-linked",
  ]);
  if (
    (preEventStages.has(stage) && events.length !== 0) ||
    (stage === "authority-held" &&
      canonical(kinds) !== canonical(["start-authority"])) ||
    (stage === "attempt-fenced" && !kinds.includes("start-attempt")) ||
    (stage === "witness-unlinked" &&
      (kinds.includes("start-attempt") || kinds.includes("completion"))) ||
    (stage === "completion-retired" && kinds.at(-1) !== "completion")
  ) {
    fail("live provider effect recovery event stage was refused");
  }
}

function providerEffectCurrentStage(stage, witness, events, previousStage) {
  if (stage !== undefined && events.length !== 0) {
    fail("provider effect event preceded witness publication");
  }
  if (stage !== undefined && witness !== undefined) return "witness-linked";
  if (stage !== undefined) {
    if (stage.initializing === true) return "stage-initializing";
    return stage.metadata.nlink === 1n ? "stage-only" : "marker-linked";
  }
  if (witness !== undefined) {
    if (events.at(-1)?.value.event_kind === "completion") {
      if (witness.metadata.nlink !== 1n) {
        fail("completed provider effect retained its marker");
      }
      return "completion-retired";
    }
    if (events.some((event) => event.value.event_kind === "start-attempt")) {
      if (witness.metadata.nlink !== 2n) {
        fail("provider effect attempt fence lost its marker");
      }
      return "attempt-fenced";
    }
    if (witness.metadata.nlink === 1n) return "witness-unlinked";
    if (events.some((event) => event.value.event_kind === "start-authority")) {
      return "authority-held";
    }
    return "marker-held";
  }
  if (events.length !== 0) {
    fail("provider effect event lacked its witness");
  }
  return previousStage === "stage-initializing" ||
    previousStage === "stage-only" ||
    previousStage === "stage-retired-before-effect"
    ? "stage-retired-before-effect"
    : "not-started";
}

function providerEffectEventPrefixCount(events, headSha256, witnessSha256) {
  if (headSha256 === ZERO_SHA256 || headSha256 === witnessSha256) return 0;
  const index = events.findIndex((event) => digest(event.bytes) === headSha256);
  if (index === -1) {
    fail("live provider effect recovery event frontier was refused");
  }
  return index + 1;
}

function providerEffectPublisherAuthorityChain(
  events,
  recoveries,
  witness,
  { includePending = false } = {},
) {
  const witnessSha256 = digest(witness.bytes);
  const authorities = [{
    authority_sha256: witness.value.slot_sha256,
    first_event_sequence: 0,
    kind: "owner-slot",
    prior_authority_sha256: ZERO_SHA256,
    recovery_claim_sha256: ZERO_SHA256,
  }];
  for (const recovery of recoveries) {
    const firstEventSequence = providerEffectEventPrefixCount(
      events,
      recovery.value.observed_evidence_head_sha256,
      witnessSha256,
    );
    if (
      firstEventSequence > events.length ||
      (!includePending && firstEventSequence === events.length)
    ) {
      continue;
    }
    const authoritySha256 = digest(recovery.bytes);
    authorities.push({
      authority_sha256: authoritySha256,
      first_event_sequence: firstEventSequence,
      kind: "state-recovery-claim",
      prior_authority_sha256: authorities.at(-1).authority_sha256,
      recovery_claim_sha256: authoritySha256,
    });
  }
  return authorities;
}

function validateLiveProviderEffectRecoveryHistory(
  effectState,
  recoveries,
) {
  const { events, slot, stage, witness } = effectState;
  const witnessSha256 =
    witness !== undefined
      ? digest(witness.bytes)
      : stage?.initializing === true
        ? providerEffectInitializingStageSha256(stage, slot)
        : stage !== undefined
          ? digest(stage.bytes)
          : ZERO_SHA256;
  let previousStage;
  let previousEventCount = 0;
  let historicalWitnessSha256 = witnessSha256;
  for (const recovery of recoveries) {
    const recoveryStage = recovery.value.observed_evidence_stage;
    const descriptor = PROVIDER_EFFECT_RECOVERY_STAGES[recoveryStage];
    if (descriptor === undefined) {
      fail("live provider effect recovery history was refused");
    }
    if (
      recoveryStage !== "not-started" &&
      recovery.value.observed_evidence_head_sha256 === ZERO_SHA256
    ) {
      fail("live provider effect recovery history was refused");
    }
    if (
      historicalWitnessSha256 === ZERO_SHA256 &&
      recoveryStage !== "not-started"
    ) {
      historicalWitnessSha256 =
        recovery.value.observed_evidence_head_sha256;
    }
    const eventCount = providerEffectEventPrefixCount(
      events,
      recovery.value.observed_evidence_head_sha256,
      historicalWitnessSha256,
    );
    const markerPresent = new Set([
      "attempt-fenced",
      "authority-held",
      "marker-held",
      "marker-linked",
      "witness-linked",
    ]).has(recoveryStage);
    validateProviderEffectStageEventConsistency(
      recoveryStage,
      events.slice(0, eventCount),
    );
    const expectedTopologySha256 =
      recoveryStage === "not-started"
        ? ZERO_SHA256
        : effectRecoveryTopologySha256({
            eventCount,
            eventHeadSha256:
              eventCount === 0
                ? ZERO_SHA256
                : digest(events[eventCount - 1].bytes),
            evidenceStage: recoveryStage,
            localLinks: descriptor.links,
            markerPresent,
            slotSequence: recovery.value.slot_sequence,
            witnessSha256: historicalWitnessSha256,
          });
    if (
      eventCount < previousEventCount ||
      (previousStage === undefined &&
        recoveryStage === "stage-retired-before-effect") ||
      (previousStage !== undefined &&
        !liveProviderEffectRecoveryStageReachable(
          previousStage,
          recoveryStage,
        )) ||
      recovery.value.observed_evidence_prefix_sha256 !==
        expectedTopologySha256 ||
      recovery.value.observed_residual_sha256 !== expectedTopologySha256 ||
      (eventCount === 0
        ? recoveryStage === "not-started"
          ? recovery.value.observed_evidence_head_sha256 !== ZERO_SHA256
          : recovery.value.observed_evidence_head_sha256 !==
            historicalWitnessSha256
        : recovery.value.observed_evidence_head_sha256 !==
          digest(events[eventCount - 1].bytes))
    ) {
      fail("live provider effect recovery history was refused");
    }
    previousStage = recoveryStage;
    previousEventCount = eventCount;
  }
  const currentStage = providerEffectCurrentStage(
    stage,
    witness,
    events,
    previousStage,
  );
  validateProviderEffectStageEventConsistency(currentStage, events);
  if (
    previousStage !== undefined &&
    !liveProviderEffectRecoveryStageReachable(previousStage, currentStage)
  ) {
    fail("live provider effect recovery topology regressed");
  }
  return currentStage;
}

function loadState(
  roots,
  checkSource,
  allowCompetingStaging = false,
  beforeMutationStageValidationObserver,
) {
  const entries = readdirSync(roots.stateBase).sort();
  if (
    entries.length > MAX_INERT_STAGING + 2 ||
    !entries.includes("active") ||
    entries.some((entry) =>
      entry !== "active" && !/^\.(?:pending|run)-[0-9a-f]{32}$/.test(entry),
    )
  ) {
    fail("state base inventory was refused");
  }
  const activePlan = parseCanonical(roots.active, "active plan receipt", 2);
  if (!onlyLowerHex(activePlan.value?.fixture_id, 32)) fail("active plan identity was refused");
  const runName = `.run-${activePlan.value.fixture_id}`;
  if (!entries.includes(runName)) fail("state base inventory was refused");
  const run = join(roots.stateBase, runName);
  const { inventory, stateMetadata } = validatePlanRunInventory(
    run,
    beforeMutationStageValidationObserver,
  );
  const candidate = parseCanonical(join(run, "candidate.json"), "candidate");
  validateCandidate(candidate.value);
  const plan = parseCanonical(join(run, "00-plan.json"), "plan receipt", 2);
  if (!plan.bytes.equals(activePlan.bytes)) fail("active plan receipt identity was refused");
  const activeMetadata = exactLstat(roots.active);
  const planMetadata = exactLstat(join(run, "00-plan.json"));
  if (
    activeMetadata.dev !== planMetadata.dev ||
    activeMetadata.ino !== planMetadata.ino ||
    activeMetadata.nlink !== 2n ||
    planMetadata.nlink !== 2n
  ) {
    fail("active plan receipt link was refused");
  }
  validatePlan(plan.value, candidate.bytes, stateMetadata);
  if (candidate.value.run_id !== plan.value.fixture_id) fail("state identity was refused");
  if (
    inventory.mutationSlots.some(
      (slot) => slot.value.fixture_id !== candidate.value.run_id,
    ) ||
    inventory.mutationCloses.some(
      (close) => close.value.fixture_id !== candidate.value.run_id,
    ) ||
    inventory.allMutationRecoveries.some(
      (recovery) => recovery.value.fixture_id !== candidate.value.run_id,
    ) ||
    inventory.mutationOperations.some(
      (operation) => operation.value.fixture_id !== candidate.value.run_id,
    )
  ) {
    fail("mutation journal identity was refused");
  }
  const receipts = [];
  for (const [index, name] of inventory.receipts.entries()) {
    const expectedLinks =
      index === 0 || name === inventory.pendingPublication?.linkedReceipt ? 2 : 1;
    const parsed =
      index === 0
        ? plan
        : parseCanonical(join(run, name), "phase receipt", expectedLinks);
    if (name !== receiptFileName(parsed.value)) fail("phase receipt filename was refused");
    receipts.push(parsed.value);
  }
  let receiptState;
  try {
    receiptState = validateReceiptChain(receipts, candidate.value.run_id);
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  const mutationClosesBySlot = new Map(
    inventory.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  const receiptSequencesBySha256 = new Map(
    receipts.map((receipt) => [digest(canonicalBytes(receipt)), receipt.sequence]),
  );
  const authoritativeRecoveriesBySlot = new Map();
  for (const [sequence, slot] of inventory.mutationSlots.entries()) {
    const lease = slot.value;
    const sourceReceipt = receipts[lease.source_sequence];
    const providerIntentReceipt = receipts[lease.source_sequence + 1];
    const controlledCleanup =
      lease.action === "provider-cleanup" &&
      lease.operation_kind === CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND;
    if (
      sourceReceipt === undefined ||
      digest(canonicalBytes(sourceReceipt)) !== lease.source_head_sha256 ||
      (sequence === 0 &&
        (lease.source_sequence !== 0 ||
          lease.source_head_sha256 !== digest(canonicalBytes(plan.value)))) ||
      (lease.action === "provider-create" &&
        (lease.source_sequence !== 0 ||
          lease.intent_receipt_sha256 === ZERO_SHA256 ||
          (providerIntentReceipt !== undefined &&
            !(
              providerIntentReceipt.phase === "preflight-refused" ||
              (providerIntentReceipt.phase === "provider-create-intent" &&
                digest(canonicalBytes(providerIntentReceipt)) === lease.intent_receipt_sha256)
            )))) ||
      (controlledCleanup &&
        (sourceReceipt.phase !== "project-cleanup-passed" ||
          lease.intent_receipt_sha256 === ZERO_SHA256 ||
          (providerIntentReceipt !== undefined &&
            (providerIntentReceipt.phase !== "provider-cleanup-intent" ||
              digest(canonicalBytes(providerIntentReceipt)) !==
                lease.intent_receipt_sha256)))) ||
      (lease.action !== "provider-create" &&
        !controlledCleanup &&
        lease.intent_receipt_sha256 !== ZERO_SHA256)
    ) {
      fail("mutation slot receipt binding was refused");
    }
    if (
      lease.action === LIVE_PROVIDER_PLAN_ACTION &&
      (lease.source_sequence !== 0 ||
        lease.source_head_sha256 !== digest(plan.bytes) ||
        lease.source_environment_sha256 !== ZERO_SHA256 ||
        lease.operation_plan.source_sequence !== 0 ||
        lease.operation_plan.source_head_sha256 !== digest(plan.bytes) ||
        lease.operation_plan.source_candidate_sha256 !== digest(candidate.bytes) ||
        lease.operation_plan.fixture_id !== candidate.value.run_id ||
        lease.operation_plan.provider_resource !== plan.value.result.provider_resource)
    ) {
      fail("live provider plan state binding was refused");
    }
    if (
      sequence === 0 &&
      (lease.previous_close_sha256 !== ZERO_SHA256 ||
        lease.source_environment_sha256 !== ZERO_SHA256)
    ) {
      fail("mutation slot journal root was refused");
    }
    if (sequence > 0) {
      const previousClose = mutationClosesBySlot.get(sequence - 1);
      if (
        previousClose === undefined ||
        lease.previous_close_sha256 !== digest(previousClose.bytes) ||
        lease.source_environment_sha256 !== previousClose.value.result_environment_sha256 ||
        lease.source_sequence !== previousClose.value.result_sequence ||
        lease.source_head_sha256 !== previousClose.value.result_head_sha256
      ) {
        fail("mutation slot journal continuity was refused");
      }
    }
    const close = mutationClosesBySlot.get(sequence);
    if (close === undefined) continue;
    const resultReceipt = receipts[close.value.result_sequence];
    const resultDelta = close.value.result_sequence - lease.source_sequence;
    const boundOperation = inventory.operationsBySlot.get(sequence);
    const effectEvents = inventory.effectEventsBySlot.get(sequence) ?? [];
    const effectCompletion =
      effectEvents.at(-1)?.value.event_kind === "completion"
        ? effectEvents.at(-1)
        : undefined;
    const effectTerminalReceipt = effectEvents.find(
      (event) => event.value.event_kind === "terminal-receipt",
    );
    const expectedOperationEvidenceSha256 =
      boundOperation !== undefined
        ? digest(boundOperation.bytes)
        : effectCompletion === undefined
          ? ZERO_SHA256
          : digest(effectCompletion.bytes);
    const slotRecoveries = inventory.allMutationRecoveries.filter(
      (recovery) => recovery.value.slot_sequence === sequence,
    );
    const authoritativeRecoveries = authoritativeRecoveriesAtClose(
      slotRecoveries,
      close,
      slot,
    );
    const authorityValid = authoritativeRecoveries !== undefined;
    if (authorityValid) {
      authoritativeRecoveriesBySlot.set(sequence, authoritativeRecoveries);
    }
    if (
      resultReceipt === undefined ||
      digest(canonicalBytes(resultReceipt)) !== close.value.result_head_sha256 ||
      !authorityValid ||
      ((lease.action !== "finalize-environment" ||
        close.value.disposition === "aborted-before-effect") &&
        close.value.result_environment_sha256 !== lease.source_environment_sha256) ||
      (close.value.disposition === "aborted-before-effect" &&
        (close.value.result_sequence !== lease.source_sequence ||
          close.value.result_head_sha256 !== lease.source_head_sha256 ||
          close.value.operation_evidence_sha256 !== ZERO_SHA256)) ||
      (close.value.disposition === "completed" &&
        lease.action === "provider-create" &&
        (!new Set([1, 2, 3]).has(resultDelta) ||
          (resultDelta === 1 && resultReceipt.phase !== "preflight-refused") ||
          (resultDelta > 1 && providerIntentReceipt === undefined) ||
          (resultDelta === 2 &&
            !new Set([
              "execution-failed",
              "provider-create-failed",
              "provider-create-passed",
            ]).has(resultReceipt.phase)) ||
          (resultDelta === 3 &&
            (receipts[lease.source_sequence + 2]?.phase !== "provider-create-passed" ||
              resultReceipt.phase !== "execution-failed")))) ||
      (close.value.disposition === "completed" &&
        lease.action === "finalize-environment" &&
        (!new Set([0, 1]).has(resultDelta) ||
          resultReceipt.phase !== "finalize-passed" ||
          (resultDelta === 0 && sourceReceipt.phase !== "finalize-passed") ||
          close.value.result_environment_sha256 === ZERO_SHA256)) ||
      (close.value.disposition === "completed" &&
        controlledCleanup &&
        (resultDelta !== 2 ||
          providerIntentReceipt?.phase !== "provider-cleanup-intent" ||
          resultReceipt.phase !== "provider-cleanup-passed")) ||
      (close.value.disposition === "completed" &&
        new Set([
          COLIMA_LIVE_PROVIDER_INTENT_ACTION,
          COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
          COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
        ]).has(lease.action) &&
        resultDelta !== 0) ||
      (close.value.disposition === "completed" &&
        lease.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
        (effectCompletion === undefined ||
          (effectCompletion.value.evidence?.variant ===
          "pre-attempt-retired-zero-receipt"
            ? resultDelta !== 0
            : effectTerminalReceipt === undefined ||
              resultDelta !== 1 ||
              resultReceipt.phase !== "provider-effect-retired" ||
              !canonicalBytes(resultReceipt).equals(
                canonicalBytes(effectTerminalReceipt.value.evidence.receipt),
              )))) ||
      (close.value.disposition === "completed" &&
        !controlledCleanup &&
        new Set(["append-receipt", "provider-cleanup"]).has(lease.action) &&
        (resultDelta > 2 ||
          (resultDelta === 2 && !resultReceipt.phase.endsWith("-failed")))) ||
      close.value.operation_evidence_sha256 !==
        expectedOperationEvidenceSha256
    ) {
      fail("mutation close receipt binding was refused");
    }
  }
  const lastRecoverySourceBySlot = new Map();
  for (const recovery of inventory.allMutationRecoveries) {
    const slot = inventory.mutationSlots[recovery.value.slot_sequence];
    const close = mutationClosesBySlot.get(recovery.value.slot_sequence);
    const sourceSequence = receiptSequencesBySha256.get(recovery.value.source_head_sha256);
    if (
      sourceSequence === undefined ||
      sourceSequence < slot.value.source_sequence ||
      (close !== undefined && sourceSequence > close.value.result_sequence) ||
      sourceSequence < (lastRecoverySourceBySlot.get(recovery.value.slot_sequence) ?? 0)
    ) {
      fail("mutation recovery receipt binding was refused");
    }
    lastRecoverySourceBySlot.set(recovery.value.slot_sequence, sourceSequence);
  }
  const latestMutationSlot = inventory.mutationSlots.at(-1);
  const latestMutationClose =
    latestMutationSlot === undefined
      ? undefined
      : mutationClosesBySlot.get(latestMutationSlot.value.journal_sequence);
  if (latestMutationSlot === undefined && receiptState.head.sequence !== 0) {
    fail("receipt head was outside the mutation journal");
  }
  if (
    latestMutationClose !== undefined &&
    (latestMutationClose.value.result_sequence !== receiptState.head.sequence ||
      latestMutationClose.value.result_head_sha256 !== receiptState.head_sha256)
  ) {
    fail("closed mutation journal did not cover the receipt head");
  }
  if (inventory.mutationLease !== undefined) {
    const lease = inventory.mutationLease.value;
    const delta = receiptState.head.sequence - lease.source_sequence;
    const openEffectTerminalReceipt = (
      inventory.effectEventsBySlot.get(lease.journal_sequence) ?? []
    ).find((event) => event.value.event_kind === "terminal-receipt");
    const controlledCleanup =
      lease.action === "provider-cleanup" &&
      lease.operation_kind === CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND;
    if (
      delta < 0 ||
      (controlledCleanup &&
        (delta > 2 ||
          (delta === 1 &&
            receiptState.head.phase !== "provider-cleanup-intent") ||
          (delta === 2 &&
            receiptState.head.phase !== "provider-cleanup-passed"))) ||
      (!controlledCleanup &&
        new Set(["append-receipt", "provider-cleanup"]).has(lease.action) &&
        (delta > 2 ||
          (delta === 2 && !receiptState.head.phase.endsWith("-failed")))) ||
      (lease.action === "provider-create" &&
        (delta > 3 ||
          (delta === 1 &&
            !new Set(["preflight-refused", "provider-create-intent"]).has(
              receiptState.head.phase,
            )) ||
          (delta === 2 &&
            !new Set([
              "execution-failed",
              "provider-create-failed",
              "provider-create-passed",
            ]).has(receiptState.head.phase)) ||
          (delta === 3 &&
            (receipts[lease.source_sequence + 2]?.phase !== "provider-create-passed" ||
              receiptState.head.phase !== "execution-failed")))) ||
      (lease.action === "finalize-environment" &&
        (delta > 1 ||
          (delta === 1 && receiptState.head.phase !== "finalize-passed"))) ||
      (new Set([
        COLIMA_LIVE_PROVIDER_INTENT_ACTION,
        COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
        COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
      ]).has(lease.action) &&
        delta !== 0)
      ||
      (lease.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
        (delta > 1 ||
          (delta === 1 &&
            (receiptState.head.phase !== "provider-effect-retired" ||
              openEffectTerminalReceipt === undefined ||
              !canonicalBytes(receiptState.head).equals(
                canonicalBytes(
                  openEffectTerminalReceipt.value.evidence.receipt,
                ),
              )))))
    ) {
      fail("open mutation slot did not cover the receipt head");
    }
  }
  if (
    inventory.pendingPublication !== undefined &&
    inventory.mutationLease === undefined
  ) {
    fail("pending receipt publication was outside an open mutation slot");
  }
  if (
    inventory.environmentPublication !== undefined &&
    inventory.mutationLease?.value.action !== "finalize-environment"
  ) {
    fail("pending environment publication was outside a finalization slot");
  }
  for (const receipt of receipts) {
    let requiredAction;
    if (receipt.phase.startsWith("provider-create-")) requiredAction = "provider-create";
    if (receipt.phase.startsWith("provider-cleanup-")) requiredAction = "provider-cleanup";
    if (receipt.phase === "provider-effect-retired") {
      requiredAction = COLIMA_LIVE_PROVIDER_EFFECT_ACTION;
    }
    if (receipt.phase === "finalize-passed") requiredAction = "finalize-environment";
    if (requiredAction === undefined && receipt.phase !== "preflight-refused") continue;
    const owner = inventory.mutationSlots.find((slot) => {
      const close = mutationClosesBySlot.get(slot.value.journal_sequence);
      const resultSequence = close?.value.result_sequence ?? receiptState.head.sequence;
      return receipt.sequence > slot.value.source_sequence && receipt.sequence <= resultSequence;
    });
    const allowedAction = owner?.value.action === (requiredAction ?? "provider-create");
    if (!allowedAction) {
      fail(`${receipt.phase} receipt was outside its mutation action`);
    }
  }
  const liveProviderPlanSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === LIVE_PROVIDER_PLAN_ACTION,
  );
  for (const slot of liveProviderPlanSlots) {
    const sequence = slot.value.journal_sequence;
    const close = mutationClosesBySlot.get(sequence);
    if (
      inventory.mutationSlots
        .slice(0, sequence)
        .some(
          (predecessor) =>
            predecessor.value.action !== LIVE_PROVIDER_PLAN_ACTION ||
            mutationClosesBySlot.get(predecessor.value.journal_sequence)?.value
              .disposition !== "aborted-before-effect",
        ) ||
      receipts.length !== 1 ||
      receiptState.head.phase !== "plan" ||
      inventory.environment !== undefined ||
      inventory.environmentPublication !== undefined ||
      inventory.pendingPublication !== undefined ||
      (close !== undefined &&
        (close.value.result_sequence !== 0 ||
          close.value.result_head_sha256 !== digest(plan.bytes) ||
          close.value.result_environment_sha256 !== ZERO_SHA256 ||
          close.value.operation_evidence_sha256 !== ZERO_SHA256 ||
          (close.value.disposition === "completed" &&
            close.value.authority !== "owner")))
    ) {
      fail("live provider plan journal was refused");
    }
  }
  const completedLiveProviderPlans = liveProviderPlanSlots.filter(
    (slot) =>
      mutationClosesBySlot.get(slot.value.journal_sequence)?.value.disposition ===
      "completed",
  );
  if (completedLiveProviderPlans.length > 1) {
    fail("live provider plan journal was ambiguous");
  }
  const completedLiveProviderPlanSlot = completedLiveProviderPlans[0];
  const liveProviderPlan =
    completedLiveProviderPlanSlot === undefined
      ? undefined
      : Object.freeze({
          close: mutationClosesBySlot.get(
            completedLiveProviderPlanSlot.value.journal_sequence,
          ),
          operationPlan: completedLiveProviderPlanSlot.value.operation_plan,
          slot: completedLiveProviderPlanSlot,
        });
  const liveProviderIntentSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_INTENT_ACTION,
  );
  if (
    liveProviderIntentSlots.length > 0 &&
    completedLiveProviderPlanSlot === undefined
  ) {
    fail("live provider intent lacked a completed plan");
  }
  if (completedLiveProviderPlanSlot !== undefined) {
    const planSequence = completedLiveProviderPlanSlot.value.journal_sequence;
    if (
      inventory.mutationSlots
        .slice(planSequence + 1)
        .some(
          (slot) =>
            !new Set([
              COLIMA_LIVE_PROVIDER_INTENT_ACTION,
              COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
              COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
              COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
            ]).has(slot.value.action),
        )
    ) {
      fail("live provider plan successor was refused");
    }
  }
  let intentFixtureOnly;
  for (const [index, slot] of liveProviderIntentSlots.entries()) {
    const fixtureOnly = liveProviderIntentFixtureOnly(
      slot.value.operation_kind,
      slot.value.operation_contract_sha256,
    );
    if (
      intentFixtureOnly !== undefined &&
      fixtureOnly !== intentFixtureOnly
    ) {
      fail("live provider intent evidence classes were mixed");
    }
    intentFixtureOnly = fixtureOnly;
    const close = mutationClosesBySlot.get(slot.value.journal_sequence);
    const publicationPlan = validateLiveProviderIntentPublicationPlanForState(
      slot.value.operation_plan,
      fixtureOnly,
    );
    const expectedProjection = buildColimaLivePlanCompletionProjectionStructure({
      operationPlanSha256: liveProviderPlanDigest(
        liveProviderPlanBytes(liveProviderPlan.operationPlan),
      ),
      planCloseSha256: digest(liveProviderPlan.close.bytes),
      planSlotSha256: digest(liveProviderPlan.slot.bytes),
      preparationObservationSha256:
        liveProviderPlan.operationPlan.preparation_observation_sha256,
    });
    if (
      !liveProviderPlanBytes(publicationPlan.provider_operation_plan).equals(
        liveProviderPlanBytes(liveProviderPlan.operationPlan),
      ) ||
      !canonicalBytes(
        publicationPlan.admission.completion_projection,
      ).equals(canonicalBytes(expectedProjection)) ||
      slot.value.source_sequence !== 0 ||
      slot.value.source_head_sha256 !== digest(plan.bytes) ||
      slot.value.source_environment_sha256 !== ZERO_SHA256 ||
      slot.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      liveProviderIntentSlots
        .slice(0, index)
        .some(
          (predecessor) =>
            mutationClosesBySlot.get(
              predecessor.value.journal_sequence,
            )?.value.disposition !== "aborted-before-effect",
        ) ||
      (close !== undefined &&
        (close.value.result_sequence !== 0 ||
          close.value.result_head_sha256 !== digest(plan.bytes) ||
          close.value.result_environment_sha256 !== ZERO_SHA256 ||
          close.value.operation_evidence_sha256 !== ZERO_SHA256 ||
          (close.value.disposition === "completed" &&
            close.value.authority !== "owner")))
    ) {
      fail("live provider intent journal was refused");
    }
  }
  const completedLiveProviderIntents = liveProviderIntentSlots.filter(
    (slot) =>
      mutationClosesBySlot.get(slot.value.journal_sequence)?.value.disposition ===
      "completed",
  );
  if (
    completedLiveProviderIntents.length > 1
  ) {
    fail("live provider intent journal was ambiguous");
  }
  const completedLiveProviderIntentSlot = completedLiveProviderIntents[0];
  const completedLiveProviderIntent =
    completedLiveProviderIntentSlot === undefined
      ? undefined
      : Object.freeze({
          close: mutationClosesBySlot.get(
            completedLiveProviderIntentSlot.value.journal_sequence,
          ),
          fixtureOnly: intentFixtureOnly,
          publicationPlan:
            completedLiveProviderIntentSlot.value.operation_plan,
          slot: completedLiveProviderIntentSlot,
        });
  const liveProviderIntent =
    completedLiveProviderIntent?.fixtureOnly === false
      ? completedLiveProviderIntent
      : undefined;
  const liveProviderFixtureIntent =
    completedLiveProviderIntent?.fixtureOnly === true
      ? completedLiveProviderIntent
      : undefined;
  const liveProviderStartDecisionSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
  );
  if (
    liveProviderStartDecisionSlots.length > 0 &&
    completedLiveProviderIntent === undefined
  ) {
    fail("live provider start decision lacked a completed intent");
  }
  if (completedLiveProviderIntent !== undefined) {
    const intentSequence =
      completedLiveProviderIntent.slot.value.journal_sequence;
    if (
      inventory.mutationSlots
        .slice(intentSequence + 1)
        .some(
          (slot) =>
            !new Set([
              COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
              COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
              COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
            ]).has(slot.value.action),
        ) ||
      liveProviderStartDecisionSlots.some(
        (slot) => slot.value.journal_sequence <= intentSequence,
      )
    ) {
      fail("live provider intent successor was refused");
    }
  }
  let startDecisionFixtureOnly;
  let startDecisionSource;
  for (const [index, slot] of liveProviderStartDecisionSlots.entries()) {
    const fixtureOnly = liveProviderStartDecisionFixtureOnly(
      slot.value.operation_kind,
      slot.value.operation_contract_sha256,
    );
    if (
      fixtureOnly !== completedLiveProviderIntent.fixtureOnly ||
      (startDecisionFixtureOnly !== undefined &&
        fixtureOnly !== startDecisionFixtureOnly)
    ) {
      fail("live provider start decision evidence classes were mixed");
    }
    startDecisionFixtureOnly = fixtureOnly;
    const close = mutationClosesBySlot.get(slot.value.journal_sequence);
    const source = Object.freeze({
      closeAuthority: completedLiveProviderIntent.close.value.authority,
      fixtureOnly,
      intentCompletion: liveProviderIntentCompletion(
        completedLiveProviderIntent,
      ),
      intentPublicationPlan: completedLiveProviderIntent.publicationPlan,
    });
    const publicationPlan =
      validateLiveProviderStartDecisionPublicationPlanForState(
        slot.value.operation_plan,
        source,
      );
    const expectedProjection =
      buildColimaLiveCompletedProviderIntentProjectionStructure(source);
    if (
      !liveProviderStartDecisionBytes(
        publicationPlan.admission.completed_intent_projection,
      ).equals(liveProviderStartDecisionBytes(expectedProjection)) ||
      slot.value.source_sequence !== 0 ||
      slot.value.source_head_sha256 !== digest(plan.bytes) ||
      slot.value.source_environment_sha256 !== ZERO_SHA256 ||
      slot.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      liveProviderStartDecisionSlots
        .slice(0, index)
        .some(
          (predecessor) =>
            mutationClosesBySlot.get(
              predecessor.value.journal_sequence,
            )?.value.disposition !== "aborted-before-effect",
        ) ||
      (close !== undefined &&
        (close.value.result_sequence !== 0 ||
          close.value.result_head_sha256 !== digest(plan.bytes) ||
          close.value.result_environment_sha256 !== ZERO_SHA256 ||
          close.value.operation_evidence_sha256 !== ZERO_SHA256 ||
          (close.value.disposition === "completed" &&
            close.value.authority !== "owner")))
    ) {
      fail("live provider start decision journal was refused");
    }
    startDecisionSource = source;
  }
  const completedLiveProviderStartDecisions =
    liveProviderStartDecisionSlots.filter(
      (slot) =>
        mutationClosesBySlot.get(slot.value.journal_sequence)?.value
          .disposition === "completed",
    );
  if (
    completedLiveProviderStartDecisions.length > 1 ||
    (completedLiveProviderStartDecisions.length === 1 &&
      inventory.mutationSlots
        .slice(
          completedLiveProviderStartDecisions[0].value.journal_sequence + 1,
        )
        .some(
          (slot) =>
            !new Set([
              COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
              COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
            ]).has(slot.value.action),
        ))
  ) {
    fail("live provider start decision journal was ambiguous");
  }
  const completedLiveProviderStartDecisionSlot =
    completedLiveProviderStartDecisions[0];
  const completedLiveProviderStartDecision =
    completedLiveProviderStartDecisionSlot === undefined
      ? undefined
      : Object.freeze({
          close: mutationClosesBySlot.get(
            completedLiveProviderStartDecisionSlot.value.journal_sequence,
          ),
          fixtureOnly: startDecisionFixtureOnly,
          publicationPlan:
            completedLiveProviderStartDecisionSlot.value.operation_plan,
          slot: completedLiveProviderStartDecisionSlot,
          source: startDecisionSource,
        });
  const liveProviderStartDecision =
    completedLiveProviderStartDecision?.fixtureOnly === false
      ? completedLiveProviderStartDecision
      : undefined;
  const liveProviderFixtureStartDecision =
    completedLiveProviderStartDecision?.fixtureOnly === true
      ? completedLiveProviderStartDecision
      : undefined;
  const liveProviderReservationSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
  );
  if (
    liveProviderReservationSlots.length > 0 &&
    completedLiveProviderStartDecision === undefined
  ) {
    fail("live provider reservation lacked a completed start decision");
  }
  const reservationSource =
    completedLiveProviderStartDecision === undefined
      ? undefined
      : Object.freeze({
          closeAuthority:
            completedLiveProviderStartDecision.close.value.authority,
          fixtureOnly: completedLiveProviderStartDecision.fixtureOnly,
          startDecisionCompletion: liveProviderStartDecisionCompletion(
            completedLiveProviderStartDecision,
          ),
          startDecisionPublicationPlan:
            completedLiveProviderStartDecision.publicationPlan,
          startDecisionSource: completedLiveProviderStartDecision.source,
        });
  let reservationFixtureOnly;
  let reservationState;
  const reservationSettlementSequences = new Set();
  for (const [index, slot] of liveProviderReservationSlots.entries()) {
    const fixtureOnly = liveProviderReservationFixtureOnly(
      slot.value.operation_kind,
      slot.value.operation_contract_sha256,
    );
    if (
      fixtureOnly !== completedLiveProviderStartDecision.fixtureOnly ||
      (reservationFixtureOnly !== undefined &&
        fixtureOnly !== reservationFixtureOnly)
    ) {
      fail("live provider reservation evidence classes were mixed");
    }
    reservationFixtureOnly = fixtureOnly;
    const publicationPlan =
      validateLiveProviderReservationPublicationPlanForState(
        slot.value.operation_plan,
        reservationSource,
      );
    const sequence = slot.value.journal_sequence;
    const close = mutationClosesBySlot.get(sequence);
    const settlement = inventory.operationsBySlot.get(sequence);
    const recoveries = inventory.allMutationRecoveries.filter(
      (recovery) => recovery.value.slot_sequence === sequence,
    );
    let expectedWitness;
    try {
      expectedWitness = buildColimaLiveProviderReservationWitness({
        publicationPlan,
        slotSequence: sequence,
        slotSha256: digest(slot.bytes),
        source: reservationSource,
      });
    } catch (error) {
      if (error instanceof LiveProviderReservationFailure) {
        fail(error.message, error.exitStatus);
      }
      throw error;
    }
    const expectedWitnessSha256 = digest(canonicalBytes(expectedWitness));
    if (
      recoveries.some(
        (recovery) =>
          recovery.value.observed_evidence_stage !== "not-started" &&
          recovery.value.observed_evidence_head_sha256 !==
            expectedWitnessSha256,
      )
    ) {
      fail("live provider reservation recovery witness was refused");
    }
    if (
      sequence <=
        completedLiveProviderStartDecision.slot.value.journal_sequence ||
      slot.value.source_sequence !== 0 ||
      slot.value.source_head_sha256 !== digest(plan.bytes) ||
      slot.value.source_environment_sha256 !== ZERO_SHA256 ||
      slot.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      liveProviderReservationSlots
        .slice(0, index)
        .some(
          (predecessor) =>
            mutationClosesBySlot.get(
              predecessor.value.journal_sequence,
            )?.value.disposition !== "aborted-before-effect",
        )
    ) {
      fail("live provider reservation journal was refused");
    }
    const stage =
      inventory.providerReservationStage?.value.slot_sequence === sequence
        ? inventory.providerReservationStage
        : undefined;
    const witness =
      inventory.providerReservationWitness?.value.slot_sequence === sequence
        ? inventory.providerReservationWitness
        : undefined;
    if (stage !== undefined) {
      try {
        validateColimaLiveProviderReservationWitness(stage.value, {
          publicationPlan,
          source: reservationSource,
        });
      } catch (error) {
        if (error instanceof LiveProviderReservationFailure) {
          fail(error.message, error.exitStatus);
        }
        throw error;
      }
    }
    if (witness !== undefined) {
      try {
        validateColimaLiveProviderReservationWitness(witness.value, {
          publicationPlan,
          source: reservationSource,
        });
      } catch (error) {
        if (error instanceof LiveProviderReservationFailure) {
          fail(error.message, error.exitStatus);
        }
        throw error;
      }
    }
    const latestRecovery = recoveries.at(-1);
    if (
      latestRecovery !== undefined &&
      !liveProviderReservationRecoveryStageReachable(
        latestRecovery.value.observed_evidence_stage,
        liveProviderReservationEvidenceStage({
          previousStage: latestRecovery.value.observed_evidence_stage,
          settlement,
          stage,
          witness,
        }),
      )
    ) {
      fail("live provider reservation recovery topology regressed");
    }
    if (settlement !== undefined) {
      if (
        stage !== undefined ||
        witness === undefined ||
        !new Set([1n, 2n]).has(witness.metadata.nlink)
      ) {
        fail("live provider reservation settlement lacked its witness");
      }
      try {
        validateColimaLiveProviderReservationSettlement(settlement.value, {
          publicationPlan,
          slotSequence: sequence,
          slotSha256: digest(slot.bytes),
          source: reservationSource,
          witness: witness.value,
        });
      } catch (error) {
        if (error instanceof LiveProviderReservationFailure) {
          fail(error.message, error.exitStatus);
        }
        throw error;
      }
      const settlementAuthorityValid =
        (settlement.value.authority === "owner" &&
          settlement.value.authority_sha256 === digest(slot.bytes) &&
          recoveries.every(
            (recovery) =>
              recovery.value.observed_settlement_sha256 ===
              digest(settlement.bytes),
          )) ||
        (settlement.value.authority === "recovery" &&
          (() => {
            const authorityIndex = recoveries.findIndex(
              (recovery) =>
                digest(recovery.bytes) === settlement.value.authority_sha256,
            );
            return (
              authorityIndex !== -1 &&
              PROVIDER_RESERVATION_SETTLEMENT_AUTHORITY_STAGES.includes(
                recoveries[authorityIndex].value.observed_evidence_stage,
              ) &&
              recoveries
                .slice(0, authorityIndex + 1)
                .every(
                  (recovery) =>
                    recovery.value.observed_settlement_sha256 === ZERO_SHA256,
                ) &&
              recoveries
                .slice(authorityIndex + 1)
                .every(
                  (recovery) =>
                    recovery.value.observed_settlement_sha256 ===
                    digest(settlement.bytes),
                )
            );
          })());
      if (!settlementAuthorityValid) {
        fail("live provider reservation settlement authority was refused");
      }
      reservationSettlementSequences.add(sequence);
    }
    if (
      close?.value.disposition === "aborted-before-effect" &&
      (stage !== undefined || witness !== undefined || settlement !== undefined)
    ) {
      fail("live provider reservation effect could not be aborted");
    }
    if (
      close?.value.disposition === "completed" &&
      (stage !== undefined ||
        witness === undefined ||
        witness.metadata.nlink !== 1n ||
        settlement === undefined ||
        close.value.operation_evidence_sha256 !== digest(settlement.bytes))
    ) {
      fail("live provider reservation completion was refused");
    }
    reservationState = Object.freeze({
      close,
      fixtureOnly,
      publicationPlan,
      recoveries,
      settlement,
      slot,
      source: reservationSource,
      stage,
      witness,
    });
  }
  if (
    inventory.providerReservationStage !== undefined &&
    reservationState?.stage === undefined
  ) {
    fail("provider reservation stage was outside the current generation");
  }
  if (
    inventory.providerReservationWitness !== undefined &&
    reservationState?.witness === undefined
  ) {
    fail("provider reservation witness was outside the current generation");
  }
  const completedLiveProviderReservations = liveProviderReservationSlots.filter(
    (slot) =>
      mutationClosesBySlot.get(slot.value.journal_sequence)?.value.disposition ===
      "completed",
  );
  if (
    completedLiveProviderReservations.length > 1 ||
    (completedLiveProviderReservations.length === 1 &&
      completedLiveProviderReservations[0] !== inventory.mutationSlots.at(-1))
  ) {
    fail("live provider reservation journal was ambiguous");
  }
  const liveProviderReservation =
    reservationState?.fixtureOnly === false ? reservationState : undefined;
  const liveProviderFixtureReservation =
    reservationState?.fixtureOnly === true ? reservationState : undefined;
  const liveProviderEffectSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
  );
  if (
    liveProviderEffectSlots.length > 0 &&
    completedLiveProviderStartDecision === undefined
  ) {
    fail("live provider effect lacked a completed start decision");
  }
  if (
    liveProviderEffectSlots.length > 0 &&
    liveProviderReservationSlots.length > 0
  ) {
    fail("live provider reservation and effect branches were mixed");
  }
  let liveProviderFixtureEffect;
  for (const [index, slot] of liveProviderEffectSlots.entries()) {
    const fixtureOnly = liveProviderEffectFixtureOnly(
      slot.value.operation_kind,
      slot.value.operation_contract_sha256,
    );
    if (
      fixtureOnly !== true ||
      completedLiveProviderStartDecision.fixtureOnly !== true
    ) {
      fail("live provider effect production persistence was refused");
    }
    const publicationPlan =
      validateLiveProviderEffectPublicationPlanForState(
        slot.value.operation_plan,
        reservationSource,
      );
    const sequence = slot.value.journal_sequence;
    const close = mutationClosesBySlot.get(sequence);
    const recoveries = inventory.allMutationRecoveries.filter(
      (recovery) => recovery.value.slot_sequence === sequence,
    );
    const stage =
      inventory.providerEffectStage?.slotSequence === sequence
        ? inventory.providerEffectStage
        : undefined;
    const witness =
      inventory.providerEffectWitness?.value.slot_sequence === sequence
        ? inventory.providerEffectWitness
        : undefined;
    const events = inventory.effectEventsBySlot.get(sequence) ?? [];
    if (
      sequence <=
        completedLiveProviderStartDecision.slot.value.journal_sequence ||
      slot.value.source_sequence !== 0 ||
      slot.value.source_head_sha256 !== digest(plan.bytes) ||
      slot.value.source_environment_sha256 !== ZERO_SHA256 ||
      slot.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      liveProviderEffectSlots
        .slice(0, index)
        .some(
          (predecessor) =>
            mutationClosesBySlot.get(
              predecessor.value.journal_sequence,
            )?.value.disposition !== "aborted-before-effect",
        )
    ) {
      fail("live provider effect journal was refused");
    }
    const artifacts = [stage, witness].filter(
      (artifact) => artifact !== undefined && artifact.initializing !== true,
    );
    for (const artifact of artifacts) {
      try {
        validateColimaLiveProviderEffectWitness(artifact.value, {
          publicationPlan,
          source: reservationSource,
        });
      } catch (error) {
        if (error instanceof LiveProviderEffectFailure) {
          fail(error.message, error.exitStatus);
        }
        throw error;
      }
      if (
        artifact.value.slot_sequence !== sequence ||
        artifact.value.slot_sha256 !== digest(slot.bytes) ||
        artifact.value.marker_witness_identity_sha256 !==
        providerEffectMarkerWitnessIdentitySha256(
          artifact.metadata,
          digest(slot.bytes),
        )
      ) {
        fail("live provider effect physical witness was refused");
      }
    }
    if (events.length > 0 && witness === undefined) {
      fail("live provider effect history lacked its witness");
    }
    let publisherAuthorityChain = [];
    if (witness !== undefined) {
      publisherAuthorityChain = providerEffectPublisherAuthorityChain(
        events,
        recoveries,
        witness,
      );
      try {
        validateColimaLiveProviderEffectEventHistory(
          events.map((event) => event.value),
          {
            publicationPlan,
            publisherAuthorityChain,
            source: reservationSource,
            witness: witness.value,
          },
        );
      } catch (error) {
        if (error instanceof LiveProviderEffectFailure) {
          fail(error.message, error.exitStatus);
        }
        throw error;
      }
      const authorityEvent = events.find(
        (event) => event.value.event_kind === "start-authority",
      );
      if (
        authorityEvent !== undefined &&
        (authorityEvent.value.publisher_authority_sha256 !==
          digest(slot.bytes) ||
          authorityEvent.value.evidence.current_owner_boot_sha256 !==
            slot.value.owner_boot_sha256 ||
          authorityEvent.value.evidence.current_owner_instance_sha256 !==
            slot.value.owner_instance_sha256)
      ) {
        fail("live provider effect owner start authority was refused");
      }
    }
    const effectState = {
      close,
      events,
      fixtureOnly,
      publicationPlan,
      publisherAuthorityChain,
      recoveries,
      slot,
      source: reservationSource,
      stage,
      witness,
    };
    const currentStage = validateLiveProviderEffectRecoveryHistory(
      effectState,
      recoveries,
    );
    const completion =
      events.at(-1)?.value.event_kind === "completion"
        ? events.at(-1)
        : undefined;
    if (
      close?.value.disposition === "aborted-before-effect" &&
      (!new Set(["not-started", "stage-retired-before-effect"]).has(
        currentStage,
      ) ||
        stage !== undefined ||
        witness !== undefined ||
        events.length !== 0 ||
        close.value.operation_evidence_sha256 !== ZERO_SHA256)
    ) {
      fail("live provider effect could not be aborted");
    }
    if (
      close?.value.disposition === "completed" &&
      (currentStage !== "completion-retired" ||
        completion === undefined ||
        witness?.metadata.nlink !== 1n ||
        close.value.operation_evidence_sha256 !== digest(completion.bytes))
    ) {
      fail("live provider effect completion was refused");
    }
    liveProviderFixtureEffect = Object.freeze({
      ...effectState,
      completion,
      currentStage,
    });
  }
  if (
    inventory.providerEffectStage !== undefined &&
    liveProviderFixtureEffect?.stage === undefined
  ) {
    fail("provider effect stage was outside the current generation");
  }
  if (
    inventory.providerEffectWitness !== undefined &&
    liveProviderFixtureEffect?.witness === undefined
  ) {
    fail("provider effect witness was outside the current generation");
  }
  if (
    inventory.providerEffectEvents.length > 0 &&
    liveProviderFixtureEffect?.events.length === 0
  ) {
    fail("provider effect events were outside the current generation");
  }
  const completedLiveProviderEffects = liveProviderEffectSlots.filter(
    (slot) =>
      mutationClosesBySlot.get(slot.value.journal_sequence)?.value.disposition ===
      "completed",
  );
  if (
    completedLiveProviderEffects.length > 1 ||
    (completedLiveProviderEffects.length === 1 &&
      completedLiveProviderEffects[0] !== inventory.mutationSlots.at(-1))
  ) {
    fail("live provider effect journal was ambiguous");
  }
  const providerSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === "provider-create",
  );
  if (providerSlots.length > 1) {
    fail("provider mutation slot was ambiguous");
  }
  const providerSlot = providerSlots[0];
  const cleanupSlots = inventory.mutationSlots.filter(
    (slot) => slot.value.action === "provider-cleanup",
  );
  const cleanupSlot = cleanupSlots.at(-1);
  const providerClose =
    providerSlot === undefined
      ? undefined
      : mutationClosesBySlot.get(providerSlot.value.journal_sequence);
  const providerIntentCandidate =
    providerSlot === undefined ? undefined : receipts[providerSlot.value.source_sequence + 1];
  const providerIntentReceipt =
    providerIntentCandidate?.phase === "provider-create-intent"
      ? providerIntentCandidate
      : undefined;
  const providerPassedCandidate =
    providerSlot === undefined ? undefined : receipts[providerSlot.value.source_sequence + 2];
  const providerPassedReceipt =
    providerPassedCandidate?.phase === "provider-create-passed"
      ? providerPassedCandidate
      : undefined;
  const providerTerminalReceipt =
    providerClose !== undefined
      ? receipts[providerClose.value.result_sequence]
      : providerSlot !== undefined &&
          inventory.mutationLease?.value.journal_sequence === providerSlot.value.journal_sequence
        ? receiptState.head
        : undefined;
  const providerEntries = readdirSync(inventory.providerDirectory).sort();
  let providerState;
  let operationSettlement;
  let cleanupSettlement;
  if (providerSlot === undefined) {
    if (
      providerEntries.length !== 0 ||
      inventory.mutationOperations.some(
        (operation) =>
          !reservationSettlementSequences.has(operation.value.slot_sequence),
      )
    ) {
      fail("provider evidence was outside a mutation slot");
    }
    providerState = Object.freeze({
      contract:
        liveProviderFixtureEffect !== undefined
          ? "live-provider-fixture-effect-only"
          : liveProviderReservation !== undefined
          ? "live-provider-reservation-only"
          : liveProviderFixtureReservation !== undefined
            ? "live-provider-fixture-reservation-only"
            : liveProviderStartDecision !== undefined
              ? "live-provider-start-decision-only"
              : liveProviderFixtureStartDecision !== undefined
                ? "live-provider-fixture-start-decision-only"
            : liveProviderIntent !== undefined
              ? "live-provider-intent-only"
              : liveProviderFixtureIntent !== undefined
                ? "live-provider-fixture-intent-only"
                : liveProviderPlan === undefined
                  ? "synchronous-fake"
                  : "live-provider-plan-only",
      operationEvidenceSha256:
        liveProviderFixtureEffect?.completion === undefined
          ? ZERO_SHA256
          : digest(liveProviderFixtureEffect.completion.bytes),
    });
  } else if (providerSlot.value.operation_kind === DETERMINISTIC_PROVIDER_OPERATION_KIND) {
    if (providerEntries.length !== 0 || inventory.mutationOperations.length !== 0) {
      fail("deterministic provider evidence was refused");
    }
    if (
      providerIntentReceipt !== undefined &&
      (providerIntentReceipt.result.operation_kind !== providerSlot.value.operation_kind ||
        providerIntentReceipt.result.operation_plan_sha256 !== ZERO_SHA256 ||
        providerIntentReceipt.result.provider_contract_sha256 !==
          providerSlot.value.operation_contract_sha256)
    ) {
      fail("deterministic provider intent binding was refused");
    }
    if (
      providerPassedReceipt !== undefined &&
      (providerPassedReceipt.result.operation_kind !== providerSlot.value.operation_kind ||
        providerPassedReceipt.result.operation_plan_sha256 !== ZERO_SHA256 ||
        providerPassedReceipt.result.provider_contract_sha256 !==
          providerSlot.value.operation_contract_sha256)
    ) {
      fail("deterministic provider result binding was refused");
    }
    providerState = Object.freeze({
      contract: "synchronous-fake",
      operationEvidenceSha256: ZERO_SHA256,
    });
  } else if (providerSlot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND) {
    const operationPlan = providerSlot.value.operation_plan;
    if (
      operationPlan.evidence_directory.path !== inventory.providerDirectory ||
      providerIntentReceipt !== undefined &&
        (providerIntentReceipt.result.operation_kind !== providerSlot.value.operation_kind ||
          providerIntentReceipt.result.operation_plan_sha256 !==
            operationPlanSha256(operationPlan) ||
          providerIntentReceipt.result.provider_contract_sha256 !==
            providerSlot.value.operation_contract_sha256 ||
          providerIntentReceipt.result.provider_resource !== operationPlan.provider_resource ||
          providerIntentReceipt.result.provider_root_key !== operationPlan.provider_root_key)
    ) {
      fail("background provider intent binding was refused");
    }
    const bindings = backgroundCreateBindings(providerSlot);
    if (
      providerIntentReceipt !== undefined &&
      canonical(providerIntentReceipt.result) !==
        canonical(backgroundProviderIntentResult(providerSlot))
    ) {
      fail("background provider intent binding was refused");
    }
    operationSettlement = inventory.operationsBySlot.get(
      providerSlot.value.journal_sequence,
    );
    const controlledCleanup =
      cleanupSlot?.value.operation_kind ===
        CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND;
    if (cleanupSlot !== undefined && !controlledCleanup) {
      fail("background provider cleanup requires dedicated ownership evidence");
    }
    let prefix;
    let createEvidence;
    try {
      if (controlledCleanup) {
        createEvidence = inspectControlledBackgroundProvider(
          inventory.providerDirectory,
          candidate.value.run_id,
          {
            expectedCreateBindings: bindings,
            revalidateCurrentToolchain: false,
          },
        );
        if (operationSettlement === undefined) {
          fail("background cleanup lacked its completed create settlement");
        }
        prefix = Object.freeze({
          effectFrontier: Object.freeze({
            disposition: operationSettlement.value.effect_disposition,
            effect: operationSettlement.value.effect_name,
          }),
          evidenceHeadSha256: operationSettlement.value.evidence_head_sha256,
          evidencePrefixSha256: operationSettlement.value.evidence_prefix_sha256,
          evidenceStage: operationSettlement.value.evidence_stage,
          pendingPublication:
            operationSettlement.value.pending_evidence_publication ?? undefined,
          residual: Object.freeze({
            controller_presence: operationSettlement.value.controller_presence,
            hostagent_presence: operationSettlement.value.hostagent_presence,
            private_publications:
              operationSettlement.value.pending_private_publications,
            root_disposition: operationSettlement.value.root_disposition,
            sockets: operationSettlement.value.sockets,
            static_root_identity_sha256:
              operationSettlement.value.static_root_identity_sha256,
          }),
          residualSha256: operationSettlement.value.residual_sha256,
        });
      } else {
        prefix = inspectControlledBackgroundProviderPrefix(
          inventory.providerDirectory,
          candidate.value.run_id,
          {
            expectedCreateBindings: bindings,
            providerBase: operationPlan.provider_base.path,
            revalidateCurrentToolchain: false,
          },
        );
        if (operationSettlement?.value.disposition === "complete-identity") {
          createEvidence = inspectControlledBackgroundProvider(
            inventory.providerDirectory,
            candidate.value.run_id,
            {
              expectedCreateBindings: bindings,
              revalidateCurrentToolchain: false,
            },
          );
        }
      }
    } catch (error) {
      if (error instanceof ProviderProcessContractFailure) fail(error.message, error.exitStatus);
      throw error;
    }
    const historicalResourceCollision =
      operationSettlement?.value.safe_code === "resource-collision";
    const historicalCollisionPrefixPermitted =
      historicalResourceCollision &&
      operationSettlement.value.evidence_head_sha256 === prefix.evidenceHeadSha256 &&
      operationSettlement.value.evidence_prefix_sha256 === prefix.evidencePrefixSha256 &&
      operationSettlement.value.evidence_stage === prefix.evidenceStage &&
      canonical(operationSettlement.value.pending_evidence_publication) ===
        canonical(prefix.pendingPublication ?? null) &&
      ((prefix.residual.root_disposition === "absent" &&
        prefix.effectFrontier.effect ===
          (prefix.pendingPublication === undefined && prefix.evidenceStage === "empty"
            ? "empty"
            : "create-authority") &&
        prefix.effectFrontier.disposition ===
          (prefix.pendingPublication !== undefined &&
          prefix.pendingPublication.disposition !== "linked-complete"
            ? "pending"
            : "complete") &&
        prefix.residual.sockets === "absent") ||
        (prefix.residual.root_disposition === "ownership-pending" &&
          prefix.effectFrontier.effect === "provider-root-collision" &&
          prefix.effectFrontier.disposition === "complete" &&
          prefix.residual.sockets === "uninspected")) &&
      prefix.residual.controller_presence === "not-started" &&
      prefix.residual.hostagent_presence === "not-started" &&
      prefix.residual.private_publications.length === 0;
    const preIntentCollision =
      providerIntentReceipt === undefined &&
      (preIntentBackgroundResourceCollision(prefix) ||
        historicalCollisionPrefixPermitted);
    if (
      providerIntentReceipt === undefined &&
      !preIntentCollision &&
      (prefix.evidenceStage !== "empty" ||
        prefix.pendingPublication !== undefined ||
        prefix.residual.root_disposition !== "absent")
    ) {
      fail("background provider evidence preceded its intent");
    }
    if (operationSettlement !== undefined) {
      const value = operationSettlement.value;
      const recoveries = providerClose === undefined
        ? inventory.allMutationRecoveries.filter(
            (recovery) =>
              recovery.value.slot_sequence === providerSlot.value.journal_sequence,
          )
        : authoritativeRecoveriesBySlot.get(providerSlot.value.journal_sequence) ?? [];
      const settlementSha256 = digest(operationSettlement.bytes);
      const authorityIndex = recoveries.findIndex(
        (recovery) => digest(recovery.bytes) === value.authority_sha256,
      );
      const authorityClaim = authorityIndex === -1 ? undefined : recoveries[authorityIndex];
      const settledObservation = settlementObservation(operationSettlement);
      const claimsBindSettlement = recoveries.every((recovery, index) => {
        const observed = claimObservation(recovery);
        if (observed.observed_settlement_sha256 === ZERO_SHA256) {
          return value.authority === "recovery" && index <= authorityIndex;
        }
        return (
          observed.observed_settlement_sha256 === settlementSha256 &&
          canonical(observed) === canonical(settledObservation) &&
          (value.authority === "owner" || index > authorityIndex)
        );
      });
      const authorityValid =
        (value.authority === "owner" &&
          value.authority_sha256 === digest(providerSlot.bytes) &&
          recoveries.every(
            (recovery) =>
              claimObservation(recovery).observed_settlement_sha256 === settlementSha256,
          )) ||
        (value.authority === "recovery" &&
          authorityClaim !== undefined &&
          authorityIndex ===
            recoveries.findLastIndex(
              (recovery) =>
                claimObservation(recovery).observed_settlement_sha256 === ZERO_SHA256,
            ) &&
          canonical(claimObservation(authorityClaim)) ===
            canonical({
              ...settledObservation,
              observed_settlement_sha256: ZERO_SHA256,
            }));
      const expectedSettlementSource =
        providerIntentReceipt === undefined
          ? providerSlot.value.source_head_sha256
          : digest(canonicalBytes(providerIntentReceipt));
      if (
        !authorityValid ||
        !claimsBindSettlement ||
        (historicalResourceCollision && !historicalCollisionPrefixPermitted) ||
        (providerIntentReceipt === undefined && !preIntentCollision) ||
        value.source_head_sha256 !== expectedSettlementSource ||
        value.evidence_head_sha256 !== prefix.evidenceHeadSha256 ||
        value.evidence_prefix_sha256 !== prefix.evidencePrefixSha256 ||
        value.evidence_stage !== prefix.evidenceStage ||
        canonical(value.pending_evidence_publication) !==
          canonical(prefix.pendingPublication ?? null) ||
        (!historicalResourceCollision &&
          (value.effect_name !== prefix.effectFrontier.effect ||
            value.effect_disposition !== prefix.effectFrontier.disposition ||
            value.static_root_identity_sha256 !==
              prefix.residual.static_root_identity_sha256 ||
            canonical(value.pending_private_publications) !==
              canonical(prefix.residual.private_publications)))
      ) {
        fail("mutation operation settlement authority was refused");
      }
    } else {
      const recoveries = inventory.allMutationRecoveries.filter(
        (recovery) => recovery.value.slot_sequence === providerSlot.value.journal_sequence,
      );
      if (
        recoveries.some(
          (recovery) =>
            claimObservation(recovery).observed_settlement_sha256 !== ZERO_SHA256,
        )
      ) {
        fail("background provider recovery observation was refused");
      }
    }
    const operationEvidenceSha256 =
      operationSettlement === undefined ? ZERO_SHA256 : digest(operationSettlement.bytes);
    if (
      providerPassedReceipt !== undefined &&
      (operationSettlement === undefined ||
        operationSettlement.value.disposition !== "complete-identity" ||
        providerPassedReceipt.result.operation_evidence_sha256 !== operationEvidenceSha256 ||
        providerPassedReceipt.result.operation_kind !== providerSlot.value.operation_kind ||
        providerPassedReceipt.result.operation_plan_sha256 !== operationPlanSha256(operationPlan) ||
        providerPassedReceipt.result.provider_contract_sha256 !==
          providerSlot.value.operation_contract_sha256)
    ) {
      fail("background provider result binding was refused");
    }
    if (providerTerminalReceipt !== undefined) {
      let expectedTerminalResult;
      if (providerTerminalReceipt.phase === "preflight-refused") {
        if (
          providerIntentReceipt !== undefined ||
          operationSettlement?.value.disposition !== "exact-residual" ||
          operationSettlement.value.safe_code !== "resource-collision"
        ) {
          fail("background provider preflight binding was refused");
        }
        expectedTerminalResult = backgroundProviderFailureResult(
          operationSettlement,
          providerTerminalReceipt.phase,
        );
      } else if (providerTerminalReceipt.phase === "provider-create-failed") {
        if (operationSettlement?.value.disposition !== "exact-residual") {
          fail("background provider failure binding was refused");
        }
        expectedTerminalResult = backgroundProviderFailureResult(operationSettlement);
      } else if (providerTerminalReceipt.phase === "provider-create-passed") {
        if (operationSettlement?.value.disposition !== "complete-identity") {
          fail("background provider pass binding was refused");
        }
        expectedTerminalResult = backgroundProviderPassedResult(
          providerSlot,
          operationSettlement,
        );
      } else if (providerTerminalReceipt.phase === "execution-failed") {
        if (
          operationSettlement?.value.disposition !== "complete-identity" ||
          !new Set(["provider-create-intent", "provider-create-passed"]).has(
            receipts[providerTerminalReceipt.sequence - 1]?.phase,
          )
        ) {
          fail("background provider execution failure binding was refused");
        }
        expectedTerminalResult = backgroundProviderFailureResult(
          operationSettlement,
          providerTerminalReceipt.phase,
        );
      }
      if (
        expectedTerminalResult !== undefined &&
        canonical(providerTerminalReceipt.result) !== canonical(expectedTerminalResult)
      ) {
        fail("background provider terminal result was refused");
      }
    }
    if (
      providerTerminalReceipt !== undefined &&
      providerTerminalReceipt.sequence > providerSlot.value.source_sequence + 1 &&
      operationSettlement === undefined
    ) {
      fail("background provider result lacked its operation settlement");
    }
    if (
      providerClose !== undefined &&
      ((providerClose.value.disposition === "completed" && operationSettlement === undefined) ||
        providerClose.value.operation_evidence_sha256 !== operationEvidenceSha256 ||
        (!controlledCleanup &&
          providerClose.value.disposition === "aborted-before-effect" &&
          (operationSettlement !== undefined ||
            prefix.evidenceStage !== "empty" ||
            prefix.pendingPublication !== undefined ||
            prefix.effectFrontier.effect !== "empty" ||
            prefix.residual.root_disposition !== "absent" ||
            prefix.residual.controller_presence !== "not-started" ||
            prefix.residual.hostagent_presence !== "not-started" ||
            prefix.residual.sockets !== "absent" ||
            prefix.residual.private_publications.length !== 0)))
    ) {
      fail("background provider close operation evidence was refused");
    }
    providerState = Object.freeze({
      bindings,
      contract: "controlled-background-fake",
      createClose: providerClose,
      createEvidence,
      createSlot: providerSlot,
      operationEvidenceSha256,
      operationPlan,
      passedReceipt: providerPassedReceipt,
      prefix,
      settlement: operationSettlement,
    });
  } else {
    fail("provider mutation operation was refused");
  }
  if (
    providerSlot?.value.operation_kind === DETERMINISTIC_PROVIDER_OPERATION_KIND &&
    providerClose !== undefined &&
    providerClose.value.operation_evidence_sha256 !== ZERO_SHA256
  ) {
    fail("deterministic provider close operation evidence was refused");
  }
  let cleanupState = Object.freeze({
    contract: "journal-only",
    operationEvidenceSha256: ZERO_SHA256,
  });
  if (
    cleanupSlot?.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
  ) {
    if (
      providerState.contract !== "controlled-background-fake" ||
      providerState.createClose === undefined ||
      providerState.createEvidence === undefined ||
      providerState.settlement === undefined
    ) {
      fail("background cleanup create authority was refused");
    }
    const expectedOperationPlan = backgroundCleanupOperationPlan({
      createClose: providerState.createClose,
      createEvidence: providerState.createEvidence,
      createSettlement: providerState.settlement,
      createSlot: providerState.createSlot,
    });
    for (const superseded of cleanupSlots.slice(0, -1)) {
      const supersededClose = mutationClosesBySlot.get(
        superseded.value.journal_sequence,
      );
      if (
        superseded.value.operation_kind !==
          CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
        canonical(superseded.value.operation_plan) !==
          canonical(expectedOperationPlan) ||
        supersededClose?.value.disposition !== "aborted-before-effect" ||
        inventory.operationsBySlot.has(superseded.value.journal_sequence)
      ) {
        fail("superseded background cleanup slot was refused");
      }
    }
    if (
      canonical(cleanupSlot.value.operation_plan) !==
      canonical(expectedOperationPlan)
    ) {
      fail("background cleanup operation plan binding was refused");
    }
    const cleanupIntentCandidate =
      receipts[cleanupSlot.value.source_sequence + 1];
    const cleanupIntentReceipt =
      cleanupIntentCandidate?.phase === "provider-cleanup-intent"
        ? cleanupIntentCandidate
        : undefined;
    const cleanupPassedCandidate =
      receipts[cleanupSlot.value.source_sequence + 2];
    const cleanupPassedReceipt =
      cleanupPassedCandidate?.phase === "provider-cleanup-passed"
        ? cleanupPassedCandidate
        : undefined;
    const cleanupClose = mutationClosesBySlot.get(
      cleanupSlot.value.journal_sequence,
    );
    const bindings = backgroundCleanupBindings(cleanupSlot);
    let prefix;
    try {
      prefix = inspectControlledBackgroundRetirementPrefix(
        expectedOperationPlan.evidence_directory.path,
        candidate.value.run_id,
        {
          expectedBindings: bindings,
          providerBase: expectedOperationPlan.provider_base.path,
        },
      );
    } catch (error) {
      if (error instanceof ProviderProcessContractFailure) {
        fail(error.message, error.exitStatus);
      }
      throw error;
    }
    cleanupSettlement = inventory.operationsBySlot.get(
      cleanupSlot.value.journal_sequence,
    );
    if (
      cleanupIntentReceipt === undefined &&
      (prefix.cleanupStage !== "not-started" ||
        cleanupSettlement !== undefined)
    ) {
      fail("background cleanup evidence preceded its intent");
    }
    if (
      cleanupIntentReceipt !== undefined &&
      canonical(cleanupIntentReceipt.result) !==
        canonical(backgroundCleanupIntentResult(cleanupSlot))
    ) {
      fail("background cleanup intent binding was refused");
    }
    if (cleanupSettlement !== undefined) {
      const value = cleanupSettlement.value;
      const recoveries = cleanupClose === undefined
        ? inventory.allMutationRecoveries.filter(
            (recovery) =>
              recovery.value.slot_sequence ===
              cleanupSlot.value.journal_sequence,
          )
        : authoritativeRecoveriesBySlot.get(
            cleanupSlot.value.journal_sequence,
          ) ?? [];
      const settlementSha256 = digest(cleanupSettlement.bytes);
      const authorityIndex = recoveries.findIndex(
        (recovery) => digest(recovery.bytes) === value.authority_sha256,
      );
      const authorityClaim =
        authorityIndex === -1 ? undefined : recoveries[authorityIndex];
      const settledObservation = settlementObservation(cleanupSettlement);
      const claimsBindSettlement = recoveries.every((recovery, index) => {
        const observed = claimObservation(recovery);
        if (observed.observed_settlement_sha256 === ZERO_SHA256) {
          return value.authority === "recovery" && index <= authorityIndex;
        }
        return (
          observed.observed_settlement_sha256 === settlementSha256 &&
          canonical(observed) === canonical(settledObservation) &&
          (value.authority === "owner" || index > authorityIndex)
        );
      });
      const authorityValid =
        (value.authority === "owner" &&
          value.authority_sha256 === digest(cleanupSlot.bytes) &&
          recoveries.every(
            (recovery) =>
              canonical(claimObservation(recovery)) ===
              canonical(settledObservation),
          )) ||
        (value.authority === "recovery" &&
          authorityClaim !== undefined &&
          authorityIndex ===
            recoveries.findLastIndex(
              (recovery) =>
                claimObservation(recovery).observed_settlement_sha256 ===
                ZERO_SHA256,
            ) &&
          canonical(claimObservation(authorityClaim)) ===
            canonical({
              ...settledObservation,
              observed_settlement_sha256: ZERO_SHA256,
            }));
      if (
        !authorityValid ||
        !claimsBindSettlement ||
        value.cleanup_plan_sha256 !== prefix.cleanupPlanSha256 ||
        value.evidence_head_sha256 !== prefix.createEvidenceHeadSha256 ||
        value.evidence_prefix_sha256 !== prefix.observationSha256 ||
        value.provider_retirement_settlement_sha256 !==
          prefix.providerSettlementSha256 ||
        value.residual_sha256 !== prefix.remainingInventorySha256 ||
        prefix.cleanupStage !== "settled" ||
        prefix.rootDisposition !== "retired" ||
        prefix.providerSettlementPublication?.disposition !== "final" ||
        prefix.pendingProgress !== undefined
      ) {
        fail("background cleanup settlement authority was refused");
      }
    } else {
      const recoveries = inventory.allMutationRecoveries.filter(
        (recovery) =>
          recovery.value.slot_sequence === cleanupSlot.value.journal_sequence,
      );
      if (
        recoveries.some(
          (recovery) =>
            claimObservation(recovery).observed_settlement_sha256 !==
            ZERO_SHA256,
        )
      ) {
        fail("background cleanup recovery observation was refused");
      }
    }
    const operationEvidenceSha256 =
      cleanupSettlement === undefined
        ? ZERO_SHA256
        : digest(cleanupSettlement.bytes);
    if (
      cleanupPassedReceipt !== undefined &&
      (cleanupSettlement === undefined ||
        canonical(cleanupPassedReceipt.result) !==
          canonical(
            backgroundCleanupPassedResult(cleanupSlot, cleanupSettlement),
          ))
    ) {
      fail("background cleanup result binding was refused");
    }
    if (
      cleanupPassedReceipt !== undefined &&
      cleanupIntentReceipt === undefined
    ) {
      fail("background cleanup pass lacked its intent");
    }
    if (
      cleanupClose !== undefined &&
      ((cleanupClose.value.disposition === "completed" &&
        (cleanupSettlement === undefined ||
          cleanupPassedReceipt === undefined)) ||
        cleanupClose.value.operation_evidence_sha256 !==
          operationEvidenceSha256 ||
        (cleanupClose.value.disposition === "aborted-before-effect" &&
          (cleanupSettlement !== undefined ||
            cleanupIntentReceipt !== undefined ||
            prefix.cleanupStage !== "not-started")))
    ) {
      fail("background cleanup close operation evidence was refused");
    }
    cleanupState = Object.freeze({
      bindings,
      close: cleanupClose,
      contract: "controlled-background-retirement",
      intentReceipt: cleanupIntentReceipt,
      operationEvidenceSha256,
      operationPlan: expectedOperationPlan,
      passedReceipt: cleanupPassedReceipt,
      prefix,
      settlement: cleanupSettlement,
      slot: cleanupSlot,
    });
  } else if (
    cleanupSlot !== undefined &&
    providerState.contract === "controlled-background-fake"
  ) {
    fail("background provider cleanup requires dedicated ownership evidence");
  }
  if (
    providerState.contract === "controlled-background-fake" &&
    receipts.some(
      (receipt) =>
        (receipt.phase.startsWith("provider-cleanup-") &&
          cleanupState.contract !== "controlled-background-retirement") ||
        receipt.phase.startsWith("failure-cleanup-") ||
        receipt.phase === "finalize-passed",
    )
  ) {
    fail("background provider cleanup requires dedicated ownership evidence");
  }
  const finalized = receiptState.head.phase === "finalize-passed";
  const manifestReceipts = finalized ? receipts.slice(0, -1) : receipts;
  let manifestState;
  try {
    manifestState = validateReceiptChain(manifestReceipts, candidate.value.run_id);
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  if (
    (inventory.environment !== undefined || inventory.environmentPublication !== undefined) &&
    !manifestState.manifest_eligible
  ) {
    fail("environment manifest publication was not eligible");
  }
  let environment;
  if (inventory.environment !== undefined) {
    const expectedLinks = inventory.environmentPublication?.linkedEnvironment ? 2 : 1;
    environment = parseCanonical(inventory.environment, "environment manifest", expectedLinks);
    let expected;
    try {
      expected = canonicalBytes(
        buildEnvironmentManifest(candidate.value, candidate.bytes, manifestReceipts),
      );
    } catch (error) {
      if (error instanceof ReceiptFailure) fail(error.message);
      throw error;
    }
    if (!environment.bytes.equals(expected)) fail("environment manifest content was refused");
    if (
      finalized &&
      receiptState.head.result.environment_manifest_sha256 !== digest(environment.bytes)
    ) {
      fail("final environment manifest digest was refused");
    }
  } else if (finalized) {
    fail("final environment manifest was unavailable", 69);
  }
  const environmentSha256 =
    environment === undefined ? ZERO_SHA256 : digest(environment.bytes);
  if (latestMutationSlot === undefined && environmentSha256 !== ZERO_SHA256) {
    fail("environment manifest was outside the mutation journal");
  }
  if (
    latestMutationClose !== undefined &&
    latestMutationClose.value.result_environment_sha256 !== environmentSha256
  ) {
    fail("closed mutation journal did not cover the environment manifest");
  }
  if (
    inventory.mutationLease !== undefined &&
    ((inventory.mutationLease.value.action !== "finalize-environment" &&
      inventory.mutationLease.value.source_environment_sha256 !== environmentSha256) ||
      (inventory.mutationLease.value.action === "finalize-environment" &&
        inventory.mutationLease.value.source_environment_sha256 !== ZERO_SHA256 &&
        inventory.mutationLease.value.source_environment_sha256 !== environmentSha256))
  ) {
    fail("open mutation slot did not cover the environment manifest");
  }
  validateProxyTemplate(join(run, "client", "proxy-template.json"));
  if (!allowCompetingStaging) {
    for (const entry of entries) {
      if (entry !== "active" && entry !== runName) {
        validateInertStaging(join(roots.stateBase, entry));
      }
    }
  }
  if (checkSource) {
    const current = sourceClosure(roots.repoRoot);
    if (canonical(current) !== canonical(candidate.value.source)) fail("source closure changed");
  }
  return {
    candidate: candidate.value,
    candidateBytes: candidate.bytes,
    environment,
    environmentPublication: inventory.environmentPublication,
    liveProviderFixtureIntent,
    liveProviderFixtureEffect,
    liveProviderFixtureReservation,
    liveProviderFixtureStartDecision,
    liveProviderIntent,
    liveProviderPlan,
    liveProviderReservation,
    liveProviderStartDecision,
    plan: plan.value,
    receiptState,
    receipts,
    run,
    allMutationRecoveries: inventory.allMutationRecoveries,
    mutationCloses: inventory.mutationCloses,
    mutationLease: inventory.mutationLease,
    mutationOperations: inventory.mutationOperations,
    mutationRecoveries: inventory.mutationRecoveries,
    mutationSlots: inventory.mutationSlots,
    mutationStages: inventory.mutationStages,
    providerEffectEvents: inventory.providerEffectEvents,
    providerEffectStage: inventory.providerEffectStage,
    providerEffectWitness: inventory.providerEffectWitness,
    providerReservationStage: inventory.providerReservationStage,
    providerReservationWitness: inventory.providerReservationWitness,
    pendingPublication: inventory.pendingPublication,
    operationSettlement,
    cleanupSettlement,
    cleanupState,
    providerState,
  };
}

function plan(roots, values) {
  if (values.provider !== "colima") fail("provider must be colima", 64);
  if (!privateIpv4Pool(values["ipv4-pool"])) fail("IPv4 pool must be a canonical private /24", 64);
  const initialEntries = readdirSync(roots.stateBase);
  if (initialEntries.includes("active")) fail("an active clean-engine plan already exists", 73);
  if (initialEntries.length > MAX_INERT_STAGING) fail("inert staging limit was exceeded", 73);
  for (const entry of initialEntries) {
    if (!/^\.(?:pending|run)-[0-9a-f]{32}$/.test(entry)) {
      fail("state base inventory was refused");
    }
    validateInertStaging(join(roots.stateBase, entry));
  }
  const runId = randomBytes(16).toString("hex");
  const pending = join(roots.stateBase, `.pending-${runId}`);
  const run = join(roots.stateBase, `.run-${runId}`);
  let pendingCreated = false;
  let pendingIdentity;
  let runPublished = false;
  let published = false;
  try {
    mkdirSync(pending, { mode: 0o700 });
    pendingCreated = true;
    pendingIdentity = exactLstat(pending);
    for (const directory of ["client", "evidence", "provider", "registry", "runtime"]) {
      mkdirSync(join(pending, directory), { mode: 0o700 });
    }
    syncDirectory(pending);
    syncDirectory(roots.stateBase);
    const stateMetadata = ownedPrivateDirectory(pending, "pending state");
    const suffix = `acceptance-${runId.slice(0, 24)}`;
    const candidate = {
      created_at: new Date().toISOString(),
      excluded_claims: EXCLUDED_CLAIMS,
      feature: "CPR-45",
      fixtures: {
        builder_canary: "ambient-remote-inert-zero-read-v1",
        docker_proxy: "synthetic-nonsecret-v1",
        registry_authentication: "one-run-basic-bcrypt",
        registry_image: REGISTRY_IMAGE,
        registry_transport: "loopback-tls-ephemeral",
      },
      kind: "synveda-cpr45-clean-engine-candidate",
      requested_assertions: REQUESTED_ASSERTIONS,
      run_id: runId,
      schema_version: 1,
      selection: {
        app_host: "app.synveda.test",
        auth_host: "auth.synveda.test",
        ipv4_pool: values["ipv4-pool"],
        oidc: "bundled",
        port: 8080,
        postgres: "bundled",
        profiles: ["browser-acceptance", "demo"],
        project: `synveda-development-${suffix}`,
        project_suffix: suffix,
        runtime: "development",
        scheme: "http",
      },
      source: sourceClosure(roots.repoRoot),
    };
    validateCandidate(candidate);
    const candidateBytes = canonicalBytes(candidate);
    writeExclusive(join(pending, "candidate.json"), candidateBytes);
    writeExclusive(
      join(pending, "client", "proxy-template.json"),
      canonicalBytes(PROXY_TEMPLATE),
    );
    const receipt = {
      fixture_id: runId,
      outcome: "passed",
      phase: "plan",
      previous_sha256: ZERO_SHA256,
      result: {
        candidate_sha256: digest(candidateBytes),
        project: candidate.selection.project,
        provider: "colima",
        provider_resource: `synveda-cpr45-${runId}`,
        state_device: String(stateMetadata.dev),
        state_inode: String(stateMetadata.ino),
      },
      schema: "synveda.clean-engine.receipt.v6",
      sequence: 0,
    };
    writeExclusive(join(pending, "00-plan.json"), canonicalBytes(receipt));
    syncDirectory(pending);
    try {
      renameSync(pending, run);
    } catch {
      fail("completed run publication failed", 70);
    }
    runPublished = true;
    syncDirectory(roots.stateBase);
    try {
      linkSync(join(run, "00-plan.json"), roots.active);
    } catch {
      if (existsSync(roots.active)) fail("an active clean-engine plan already exists", 73);
      fail("active plan publication failed", 70);
    }
    published = true;
    syncDirectory(roots.stateBase);
    loadState(roots, true, true);
    process.stdout.write(
      `clean-engine: plan ${runId} prepared for ${candidate.selection.project}\n`,
    );
  } catch (error) {
    if (pendingCreated && !published) {
      try {
        const cleanupPath = runPublished ? run : pending;
        const current = exactLstat(cleanupPath);
        if (
          current.isSymbolicLink() ||
          !current.isDirectory() ||
          current.dev !== pendingIdentity.dev ||
          current.ino !== pendingIdentity.ino ||
          current.uid !== OWNER_UID
        ) {
          throw new Error("pending state identity changed");
        }
        rmSync(cleanupPath, { recursive: true, force: false });
        syncDirectory(roots.stateBase);
      } catch {
        process.stderr.write("clean-engine: failed plan state was retained for inspection\n");
      }
    } else if (published) {
      process.stderr.write("clean-engine: published plan state was retained for inspection\n");
    }
    throw error;
  }
}

function activeMutationRun(roots) {
  const active = parseCanonical(roots.active, "active plan receipt", 2);
  if (!onlyLowerHex(active.value?.fixture_id, 32)) fail("active plan identity was refused");
  const run = join(roots.stateBase, `.run-${active.value.fixture_id}`);
  ownedPrivateDirectory(run, "active run state");
  return { fixtureId: active.value.fixture_id, run };
}

function mutationHeldIdentity(path) {
  const metadata = exactLstat(path);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== OWNER_UID ||
    metadata.nlink !== 1n ||
    (metadata.mode & 0o7777n) !== 0o600n
  ) {
    fail("mutation slot identity was refused");
  }
  return metadata;
}

function sameMutationArtifact(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.uid === right.uid &&
    left.mode === right.mode &&
    left.size === right.size
  );
}

function retireUnlinkedMutationStage(run, stagePath, identity) {
  const runMetadata = ownedPrivateDirectory(run, "active run state");
  let current;
  try {
    current = inspectPendingFile(
      stagePath,
      "pending mutation publication",
      runMetadata.dev,
      new Set([1n]),
    );
  } catch (error) {
    if (mutationStageWasRemoved(stagePath)) return;
    throw error;
  }
  if (!sameMetadata(identity, current)) {
    fail("pending mutation publication identity changed");
  }
  try {
    unlinkSync(stagePath);
    syncDirectory(run);
  } catch (error) {
    if (error?.code === "ENOENT") return;
    fail("pending mutation publication retirement failed", 70);
  }
}

function mutationStageWasRemoved(stagePath) {
  try {
    exactLstat(stagePath);
    return false;
  } catch (error) {
    if (error?.code === "ENOENT") return true;
    fail("pending mutation publication was unavailable", 69);
  }
}

function mutationStageObservationTransition(stagePath, identity) {
  let current;
  try {
    current = exactLstat(stagePath);
  } catch (error) {
    if (error?.code === "ENOENT") return "removed";
    fail("pending mutation publication was unavailable", 69);
  }
  if (
    !sameMutationStageIdentity(identity, current)
  ) {
    fail("pending mutation publication identity changed");
  }
  return sameMetadata(identity, current) ? "same" : "superseded";
}

function syncMutationStageRecoveryDirectory(run) {
  try {
    syncDirectory(run);
  } catch {
    fail("pending mutation publication recovery failed", 70);
  }
}

// Publish a complete, fsynced blocker through an unguessable private staging
// name. The final name is created atomically with link(2), so interruption can
// leave only an inert one-link stage or a valid two-link final artifact.
function publishMutationBlocker(
  run,
  destinationName,
  bytes,
  {
    afterAuthorityObserver,
    afterLinkObserver,
    afterLinkMilliseconds = 0,
    beforeStageObserver,
    beforeLinkMilliseconds = 0,
    reassertAuthority,
  } = {},
) {
  if (!isMutationPublicationDestinationName(destinationName)) {
    fail("mutation publication destination was refused", 70);
  }
  for (const milliseconds of [afterLinkMilliseconds, beforeLinkMilliseconds]) {
    if (
      !Number.isSafeInteger(milliseconds) ||
      milliseconds < 0 ||
      milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds
    ) {
      fail("mutation publication hold was refused", 70);
    }
  }
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail("mutation publication authority was refused", 70);
  }
  if (
    afterAuthorityObserver !== undefined &&
    typeof afterAuthorityObserver !== "function"
  ) {
    fail("mutation publication authority observer was refused", 70);
  }
  if (afterLinkObserver !== undefined && typeof afterLinkObserver !== "function") {
    fail("mutation publication observer was refused", 70);
  }
  if (
    beforeStageObserver !== undefined &&
    typeof beforeStageObserver !== "function"
  ) {
    fail("mutation publication pre-stage observer was refused", 70);
  }
  const runMetadata = ownedPrivateDirectory(run, "active run state");
  const destinationPath = join(run, destinationName);
  if (beforeStageObserver?.() !== undefined) {
    fail("mutation publication pre-stage observer returned a value", 70);
  }
  for (let attempt = 0; attempt < MAX_MUTATION_PUBLICATION_ATTEMPTS; attempt += 1) {
    const stagePath = join(run, `${MUTATION_STAGE_PREFIX}${randomBytes(16).toString("hex")}`);
    const reassert = (witness, retireOwnStageOnFailure) => {
      try {
        if (reassertAuthority?.(witness) !== undefined) {
          fail("mutation publication authority returned a value", 70);
        }
        if (
          witness !== undefined &&
          afterAuthorityObserver?.(witness) !== undefined
        ) {
          fail("mutation publication authority observer returned a value", 70);
        }
      } catch (error) {
        if (
          retireOwnStageOnFailure &&
          witness !== undefined &&
          !mutationStageWasRemoved(stagePath)
        ) {
          retireUnlinkedMutationStage(run, stagePath, witness.identity);
        }
        throw error;
      }
    };
    writeExclusive(stagePath, bytes);
    let staged;
    try {
      staged = inspectPendingFile(
        stagePath,
        "pending mutation publication",
        runMetadata.dev,
        new Set([1n]),
      );
      if (!readPrivate(stagePath, "pending mutation publication", 1, 0n).equals(bytes)) {
        fail("pending mutation publication content changed");
      }
    } catch (error) {
      if (!mutationStageWasRemoved(stagePath)) throw error;
      reassert(undefined, false);
      continue;
    }
    const witness = Object.freeze({
      bytes,
      destinationName,
      identity: staged,
      stagePath,
    });
    holdFakeProvider(beforeLinkMilliseconds);
    reassert(witness, true);
    try {
      const currentStage = inspectPendingFile(
        stagePath,
        "pending mutation publication",
        runMetadata.dev,
        new Set([1n]),
      );
      if (
        !sameMutationArtifact(staged, currentStage) ||
        !readPrivate(stagePath, "pending mutation publication", 1, 0n).equals(bytes)
      ) {
        fail("pending mutation publication identity changed");
      }
    } catch (error) {
      if (!mutationStageWasRemoved(stagePath)) throw error;
      reassert(undefined, false);
      continue;
    }
    try {
      linkSync(stagePath, destinationPath);
    } catch (error) {
      if (error?.code === "EEXIST") {
        retireUnlinkedMutationStage(run, stagePath, staged);
        return false;
      }
      if (error?.code === "ENOENT") {
        reassert(undefined, false);
        continue;
      }
      retireUnlinkedMutationStage(run, stagePath, staged);
      fail("mutation blocker publication failed", 70);
    }
    try {
      syncDirectory(run);
      holdFakeProvider(afterLinkMilliseconds);
      const linkedDestination = exactLstat(destinationPath);
      if (
        !linkedDestination.isFile() ||
        linkedDestination.isSymbolicLink() ||
        linkedDestination.uid !== OWNER_UID ||
        linkedDestination.dev !== runMetadata.dev ||
        !new Set([1n, 2n]).has(linkedDestination.nlink) ||
        (linkedDestination.mode & 0o7777n) !== 0o600n ||
        !sameMutationArtifact(staged, linkedDestination)
      ) {
        fail("mutation blocker publication identity changed");
      }
      try {
        const linkedStage = exactLstat(stagePath);
        if (
          !linkedStage.isFile() ||
          linkedStage.isSymbolicLink() ||
          linkedStage.nlink !== 2n ||
          !sameMutationArtifact(staged, linkedStage)
        ) {
          fail("mutation blocker publication identity changed");
        }
        if (
          afterLinkObserver?.(
            Object.freeze({
              destinationName,
              destinationPath,
              stagePath,
            }),
          ) !== undefined
        ) {
          fail("mutation publication observer returned a value", 70);
        }
        unlinkSync(stagePath);
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
      syncDirectory(run);
      if (!readPrivate(destinationPath, "published mutation blocker", 1, 0n).equals(bytes)) {
        fail("mutation blocker publication content changed");
      }
    } catch (error) {
      if (error instanceof ClosedFailure) throw error;
      fail("mutation blocker publication failed", 70);
    }
    return true;
  }
  fail("mutation publication was repeatedly superseded", 73);
}

function reconcileMutationStages(
  roots,
  beforeMutationStageReconciliationObserver,
) {
  if (
    beforeMutationStageReconciliationObserver !== undefined &&
    typeof beforeMutationStageReconciliationObserver !== "function"
  ) {
    fail("mutation reconciliation test observer was refused", 64);
  }
  let supersessions = 0;
  while (true) {
    const state = loadState(roots, false);
    if (state.mutationStages.length === 0) {
      syncMutationStageRecoveryDirectory(state.run);
      return state;
    }
    const runMetadata = ownedPrivateDirectory(state.run, "active run state");
    let stageWasSuperseded = false;
    for (const stage of state.mutationStages) {
      if (beforeMutationStageReconciliationObserver?.(stage.name) !== undefined) {
        fail("mutation reconciliation test observer returned a value", 70);
      }
      const expectedLinks = stage.linkedDestination === undefined ? 1n : 2n;
      let current;
      try {
        current = inspectPendingFile(
          stage.path,
          "pending mutation publication",
          runMetadata.dev,
          new Set([expectedLinks]),
        );
        if (!sameMetadata(stage.metadata, current)) {
          fail("pending mutation publication identity changed");
        }
        if (stage.linkedDestination !== undefined) {
          const destination = inspectPendingFile(
            join(state.run, stage.linkedDestination),
            "published mutation blocker",
            runMetadata.dev,
            new Set([2n]),
          );
          if (!sameMutationArtifact(current, destination)) {
            fail("pending mutation publication link was refused");
          }
        }
      } catch (error) {
        const transition = mutationStageObservationTransition(
          stage.path,
          stage.metadata,
        );
        if (transition === "same") throw error;
        if (transition === "removed") {
          syncMutationStageRecoveryDirectory(state.run);
        }
        stageWasSuperseded = true;
        break;
      }
      try {
        unlinkSync(stage.path);
      } catch (error) {
        if (error?.code !== "ENOENT") {
          fail("pending mutation publication recovery failed", 70);
        }
        stageWasSuperseded = true;
      }
      syncMutationStageRecoveryDirectory(state.run);
      if (stageWasSuperseded) break;
    }
    if (!stageWasSuperseded) {
      const verified = loadState(roots, false);
      if (verified.mutationStages.length === 0) {
        syncMutationStageRecoveryDirectory(verified.run);
        return verified;
      }
    }
    supersessions += 1;
    if (supersessions > MAX_PLAN_RUN_INVENTORY_SUPERSESSIONS) {
      fail("mutation publication reconciliation was repeatedly superseded", 73);
    }
  }
}

function acquireMutationLease(
  roots,
  action,
  intentReceiptSha256 = ZERO_SHA256,
  expectedSource,
  publicationHolds = {},
  operation = Object.freeze({
    contractSha256: ZERO_SHA256,
    kind: "none",
    plan: null,
  }),
  testObservers = {},
) {
  if (
    testObservers === null ||
    Array.isArray(testObservers) ||
    typeof testObservers !== "object" ||
    Object.keys(testObservers).some(
      (name) =>
        !new Set([
          "afterPublishedInventorySnapshot",
          "beforeMutationStageReconciliation",
        ]).has(name),
    ) ||
    Object.values(testObservers).some(
      (observer) => observer !== undefined && typeof observer !== "function",
    )
  ) {
    fail("mutation acquisition test observers were refused", 64);
  }
  const active = activeMutationRun(roots);
  const initial = reconcileMutationStages(
    roots,
    testObservers.beforeMutationStageReconciliation,
  );
  if (
    initial.liveProviderPlan !== undefined &&
    !new Set([
      COLIMA_LIVE_PROVIDER_INTENT_ACTION,
      COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
      COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
      COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
    ]).has(action)
  ) {
    fail("live provider execution remains disabled after state planning", 73);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_INTENT_ACTION &&
    (initial.liveProviderPlan === undefined ||
      initial.liveProviderIntent !== undefined ||
      initial.liveProviderFixtureIntent !== undefined)
  ) {
    fail("live provider intent publication state was refused", 73);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION &&
    ((initial.liveProviderIntent === undefined) ===
      (initial.liveProviderFixtureIntent === undefined) ||
      initial.liveProviderStartDecision !== undefined ||
      initial.liveProviderFixtureStartDecision !== undefined)
  ) {
    fail("live provider start decision publication state was refused", 73);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
    ((initial.liveProviderStartDecision === undefined) ===
      (initial.liveProviderFixtureStartDecision === undefined) ||
      initial.mutationSlots.some(
        (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
      ) ||
      initial.liveProviderReservation?.close?.value.disposition === "completed" ||
      initial.liveProviderFixtureReservation?.close?.value.disposition ===
        "completed")
  ) {
    fail("live provider reservation state was refused", 73);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
    (initial.liveProviderFixtureStartDecision === undefined ||
      initial.liveProviderStartDecision !== undefined ||
      initial.mutationSlots.some(
        (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
      ) ||
      initial.liveProviderFixtureEffect?.close?.value.disposition ===
        "completed")
  ) {
    fail("live provider effect state was refused", 73);
  }
  if (initial.mutationRecoveries.length > 0) {
    fail("a clean-engine mutation recovery is active or abandoned", 73);
  }
  if (initial.mutationLease !== undefined) {
    const ownerState = mutationOwnerState(initial.mutationLease.value);
    if (ownerState === "current" || ownerState === "unknown") {
      fail("another clean-engine mutation is active or could not be identified", 73);
    }
    fail("an abandoned clean-engine mutation requires explicit recovery", 73);
  }
  if (!new Set([
    "append-receipt",
    "finalize-environment",
    "provider-cleanup",
    "provider-create",
    COLIMA_LIVE_PROVIDER_INTENT_ACTION,
    COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
    COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
    COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
    LIVE_PROVIDER_PLAN_ACTION,
  ]).has(action)) {
    fail("mutation action was refused", 70);
  }
  if (!onlyLowerHex(intentReceiptSha256, 64)) fail("mutation intent binding was refused", 70);
  if (
    operation === null ||
    Array.isArray(operation) ||
    typeof operation !== "object" ||
    JSON.stringify(Object.keys(operation).sort()) !==
      JSON.stringify(["contractSha256", "kind", "plan"].sort()) ||
    typeof operation.kind !== "string" ||
    !onlyLowerHex(operation.contractSha256, 64) ||
    (operation.plan !== null &&
      (Array.isArray(operation.plan) || typeof operation.plan !== "object"))
  ) {
    fail("mutation operation was refused", 70);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION &&
    liveProviderStartDecisionFixtureOnly(
      operation.kind,
      operation.contractSha256,
    ) !== (initial.liveProviderFixtureIntent !== undefined)
  ) {
    fail("live provider start decision evidence class differed", 73);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
    liveProviderReservationFixtureOnly(
      operation.kind,
      operation.contractSha256,
    ) !== (initial.liveProviderFixtureStartDecision !== undefined)
  ) {
    fail("live provider reservation evidence class differed", 73);
  }
  if (
    action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
    liveProviderEffectFixtureOnly(
      operation.kind,
      operation.contractSha256,
    ) !== true
  ) {
    fail("live provider effect production persistence was refused", 73);
  }
  const journalSequence = initial.mutationSlots.length;
  if (journalSequence >= MAX_MUTATION_SLOTS) {
    fail("mutation slot journal capacity was exhausted", 73);
  }
  if (
    expectedSource !== undefined &&
    (initial.receiptState.head.sequence !== expectedSource.sequence ||
      initial.receiptState.head_sha256 !== expectedSource.sha256)
  ) {
    fail("mutation source head changed", 73);
  }
  const owner = currentProcessIdentity();
  const previousClose = initial.mutationCloses.at(-1);
  const lease = {
    action,
    fixture_id: active.fixtureId,
    intent_receipt_sha256: intentReceiptSha256,
    journal_sequence: journalSequence,
    nonce: randomBytes(16).toString("hex"),
    operation_contract_sha256: operation.contractSha256,
    operation_kind: operation.kind,
    operation_plan: operation.plan,
    owner_boot_sha256: owner.boot_sha256,
    owner_instance_sha256: owner.instance_sha256,
    owner_pid: owner.pid,
    owner_probe: owner.probe,
    previous_close_sha256:
      previousClose === undefined ? ZERO_SHA256 : digest(previousClose.bytes),
    schema: MUTATION_SLOT_SCHEMA,
    source_environment_sha256:
      initial.environment === undefined ? ZERO_SHA256 : digest(initial.environment.bytes),
    source_head_sha256: initial.receiptState.head_sha256,
    source_sequence: initial.receiptState.head.sequence,
  };
  const leaseBytes = canonicalBytes(lease);
  validateMutationLeaseValue(lease, active.fixtureId);
  const leaseName = mutationSlotFileName(journalSequence);
  const leasePath = join(active.run, leaseName);
  if (!publishMutationBlocker(
    active.run,
    leaseName,
    leaseBytes,
    publicationHolds,
  )) {
    fail("another clean-engine mutation is active", 73);
  }
  const leaseIdentity = mutationHeldIdentity(leasePath);
  const held = Object.freeze({
    active,
    identity: leaseIdentity,
    lease,
    leaseBytes,
    path: leasePath,
  });
  const verified = loadState(
    roots,
    false,
    false,
    testObservers.afterPublishedInventorySnapshot,
  );
  if (
    verified.mutationLease === undefined ||
    !verified.mutationLease.bytes.equals(leaseBytes) ||
    verified.mutationRecoveries.length !== 0
  ) {
    fail("mutation slot publication was not durable", 70);
  }
  if (
    expectedSource !== undefined &&
    (verified.receiptState.head.sequence !== expectedSource.sequence ||
      verified.receiptState.head_sha256 !== expectedSource.sha256)
  ) {
    fail("mutation source head changed", 73);
  }
  return held;
}

function assertMutationLeaseHeld(roots, held, allowRecovery = false) {
  const current = parseCanonical(held.path, "mutation slot");
  const metadata = mutationHeldIdentity(held.path);
  if (!sameMetadata(held.identity, metadata) || !current.bytes.equals(held.leaseBytes)) {
    fail("mutation slot identity changed");
  }
  const state = loadState(roots, false);
  if (
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(held.leaseBytes) ||
    (!allowRecovery && state.mutationRecoveries.length !== 0)
  ) {
    fail("mutation slot ownership was refused", 73);
  }
  return state;
}

function operationSettlementForSlot(state, slot) {
  if (slot === undefined) return undefined;
  if (
    slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION
  ) {
    return (
      state.liveProviderReservation?.settlement ??
      state.liveProviderFixtureReservation?.settlement
    );
  }
  if (
    slot.value.action === "provider-create" &&
    slot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND
  ) {
    return state.providerState.settlement;
  }
  if (
    slot.value.action === "provider-cleanup" &&
    slot.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
  ) {
    return state.cleanupState.settlement;
  }
  return undefined;
}

function operationEvidenceForSlot(state, slot) {
  if (slot?.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION) {
    const completion = state.liveProviderFixtureEffect?.completion;
    return completion === undefined ? ZERO_SHA256 : digest(completion.bytes);
  }
  const settlement = operationSettlementForSlot(state, slot);
  return settlement === undefined ? ZERO_SHA256 : digest(settlement.bytes);
}

function publishMutationClose(
  roots,
  state,
  slot,
  disposition,
  reassertAuthority,
  publicationHolds = {},
  operationEvidenceSha256 = ZERO_SHA256,
) {
  const expectedOperationEvidenceSha256 = operationEvidenceForSlot(state, slot);
  if (operationEvidenceSha256 !== expectedOperationEvidenceSha256) {
    fail("mutation close operation evidence was refused", 70);
  }
  if (
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(slot.bytes) ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined
  ) {
    fail("mutation close ownership was refused", 73);
  }
  if (
    slot.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
    disposition === "completed"
  ) {
    assertBackgroundCleanupComplete(roots, state);
  }
  if (
    slot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND &&
    disposition === "aborted-before-effect" &&
    (state.operationSettlement !== undefined ||
      !backgroundPrefixIsEmpty(state.providerState.prefix))
  ) {
    fail("background provider close disposition was refused", 73);
  }
  if (
    slot.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
    disposition === "aborted-before-effect" &&
    (state.cleanupState.settlement !== undefined ||
      state.cleanupState.intentReceipt !== undefined ||
      state.cleanupState.prefix.cleanupStage !== "not-started")
  ) {
    fail("background cleanup close disposition was refused", 73);
  }
  if (
    slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
    disposition === "aborted-before-effect" &&
    (state.providerReservationStage !== undefined ||
      state.providerReservationWitness !== undefined ||
      operationSettlementForSlot(state, slot) !== undefined)
  ) {
    fail("live provider reservation close disposition was refused", 73);
  }
  if (
    slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
    disposition === "completed" &&
    (state.providerReservationStage !== undefined ||
      state.providerReservationWitness?.metadata.nlink !== 1n ||
      operationSettlementForSlot(state, slot) === undefined)
  ) {
    fail("live provider reservation completion was refused", 73);
  }
  if (
    slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
    disposition === "aborted-before-effect" &&
    (state.providerEffectStage !== undefined ||
      state.providerEffectWitness !== undefined ||
      state.providerEffectEvents.length !== 0 ||
      operationEvidenceSha256 !== ZERO_SHA256)
  ) {
    fail("live provider effect close disposition was refused", 73);
  }
  if (
    slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
    disposition === "completed" &&
    (state.liveProviderFixtureEffect?.currentStage !==
      "completion-retired" ||
      state.providerEffectStage !== undefined ||
      state.providerEffectWitness?.metadata.nlink !== 1n ||
      state.liveProviderFixtureEffect.completion === undefined ||
      operationEvidenceSha256 !==
        digest(state.liveProviderFixtureEffect.completion.bytes))
  ) {
    fail("live provider effect completion was refused", 73);
  }
  const result = state.receiptState;
  if (
    slot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND &&
    result.head.phase === "provider-create-passed" &&
    !sourceClosureMatches(roots, state.candidate)
  ) {
    fail("background provider source closure changed", 73);
  }
  const resultEnvironmentSha256 =
    state.environment === undefined ? ZERO_SHA256 : digest(state.environment.bytes);
  if (
    disposition === "completed" &&
    new Set([
      COLIMA_LIVE_PROVIDER_INTENT_ACTION,
      COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
    ]).has(slot.value.action) &&
    (result.head.sequence !== slot.value.source_sequence ||
      result.head_sha256 !== slot.value.source_head_sha256 ||
      resultEnvironmentSha256 !== slot.value.source_environment_sha256 ||
      operationEvidenceSha256 !== ZERO_SHA256)
  ) {
    fail("inert mutation close disposition was refused", 70);
  }
  if (
    disposition === "completed" &&
    slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
    (result.head.sequence !== slot.value.source_sequence ||
      result.head_sha256 !== slot.value.source_head_sha256 ||
      resultEnvironmentSha256 !== slot.value.source_environment_sha256 ||
      operationEvidenceSha256 === ZERO_SHA256)
  ) {
    fail("live provider reservation close disposition was refused", 70);
  }
  if (
    disposition === "completed" &&
    slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
    (result.head.sequence - slot.value.source_sequence !==
      (state.liveProviderFixtureEffect.completion.value.evidence.variant ===
      "pre-attempt-retired-zero-receipt"
        ? 0
        : 1) ||
      resultEnvironmentSha256 !== slot.value.source_environment_sha256 ||
      operationEvidenceSha256 === ZERO_SHA256)
  ) {
    fail("live provider effect close disposition was refused", 70);
  }
  if (
    disposition === "aborted-before-effect" &&
    (result.head.sequence !== slot.value.source_sequence ||
      result.head_sha256 !== slot.value.source_head_sha256 ||
      resultEnvironmentSha256 !== slot.value.source_environment_sha256)
  ) {
    fail("mutation close disposition was refused", 70);
  }
  if (
    disposition === "completed" &&
    ((slot.value.action === "provider-create" &&
      !new Set([
        "execution-failed",
        "preflight-refused",
        "provider-create-failed",
        "provider-create-passed",
      ]).has(result.head.phase)) ||
      (slot.value.action === "finalize-environment" &&
        result.head.phase !== "finalize-passed") ||
      (slot.value.action === "provider-cleanup" &&
        slot.value.operation_kind ===
          CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
        result.head.phase !== "provider-cleanup-passed"))
  ) {
    fail("mutation close result was refused", 70);
  }
  const recoveries = state.mutationRecoveries;
  const authority = recoveries.length === 0 ? "owner" : "recovery";
  const authoritySha256 =
    authority === "owner" ? digest(slot.bytes) : digest(recoveries.at(-1).bytes);
  const close = {
    authority,
    authority_sha256: authoritySha256,
    disposition,
    fixture_id: slot.value.fixture_id,
    operation_evidence_sha256: operationEvidenceSha256,
    operation_contract_sha256: slot.value.operation_contract_sha256,
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(slot.value.operation_plan),
    result_environment_sha256: resultEnvironmentSha256,
    result_head_sha256: result.head_sha256,
    result_sequence: result.head.sequence,
    schema: MUTATION_CLOSE_SCHEMA,
    slot_sequence: slot.value.journal_sequence,
    slot_sha256: digest(slot.bytes),
  };
  validateMutationCloseValue(close, slot, slot.value.fixture_id);
  const closeBytes = canonicalBytes(close);
  const closeName = mutationCloseFileName(slot.value.journal_sequence);
  const reassertClosePublication = (publicationStage) => {
    const current = reassertAuthority(publicationStage);
    const currentEnvironmentSha256 =
      current.environment === undefined ? ZERO_SHA256 : digest(current.environment.bytes);
    const currentOperationEvidenceSha256 = operationEvidenceForSlot(current, slot);
    if (
      slot.value.operation_kind ===
        CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
      close.disposition === "completed"
    ) {
      assertBackgroundCleanupComplete(roots, current);
    }
    if (
      current.mutationLease === undefined ||
      !current.mutationLease.bytes.equals(slot.bytes) ||
      current.pendingPublication !== undefined ||
      current.environmentPublication !== undefined ||
      current.receiptState.head.sequence !== close.result_sequence ||
      current.receiptState.head_sha256 !== close.result_head_sha256 ||
      currentEnvironmentSha256 !== close.result_environment_sha256 ||
      currentOperationEvidenceSha256 !== close.operation_evidence_sha256 ||
      (slot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND &&
        close.disposition === "completed" &&
        current.receiptState.head.phase === "provider-create-passed" &&
        !sourceClosureMatches(roots, current.candidate)) ||
      (slot.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND &&
        close.disposition === "aborted-before-effect" &&
        (current.operationSettlement !== undefined ||
          !backgroundPrefixIsEmpty(current.providerState.prefix))) ||
      (slot.value.operation_kind ===
          CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
        close.disposition === "aborted-before-effect" &&
        (current.cleanupState.settlement !== undefined ||
          current.cleanupState.intentReceipt !== undefined ||
          current.cleanupState.prefix.cleanupStage !== "not-started"))
      ||
      (slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
        close.disposition === "completed" &&
        (current.providerReservationStage !== undefined ||
          current.providerReservationWitness?.metadata.nlink !== 1n ||
          operationSettlementForSlot(current, slot) === undefined)) ||
      (slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION &&
        close.disposition === "aborted-before-effect" &&
        (current.providerReservationStage !== undefined ||
          current.providerReservationWitness !== undefined ||
          operationSettlementForSlot(current, slot) !== undefined)) ||
      (slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
        close.disposition === "completed" &&
        (current.liveProviderFixtureEffect?.currentStage !==
          "completion-retired" ||
          current.providerEffectStage !== undefined ||
          current.providerEffectWitness?.metadata.nlink !== 1n ||
          current.liveProviderFixtureEffect.completion === undefined)) ||
      (slot.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
        close.disposition === "aborted-before-effect" &&
        (current.providerEffectStage !== undefined ||
          current.providerEffectWitness !== undefined ||
          current.providerEffectEvents.length !== 0))
    ) {
      fail("mutation close authority changed", 73);
    }
  };
  const published = publishMutationBlocker(state.run, closeName, closeBytes, {
    ...publicationHolds,
    reassertAuthority: reassertClosePublication,
  });
  if (!published) {
    const existing = parseCanonical(join(state.run, closeName), "mutation close");
    if (!existing.bytes.equals(closeBytes)) {
      fail("mutation close publication conflicted", 73);
    }
  }
  const verified = loadState(roots, false);
  const recorded = verified.mutationCloses.find(
    (candidate) => candidate.value.slot_sequence === slot.value.journal_sequence,
  );
  if (
    recorded === undefined ||
    !recorded.bytes.equals(closeBytes)
  ) {
    fail("mutation close publication was not durable", 70);
  }
  return verified;
}

function closeMutationLease(
  roots,
  held,
  disposition,
  publicationHolds = {},
  operationEvidenceSha256 = ZERO_SHA256,
  additionalReassertAuthority,
) {
  if (
    additionalReassertAuthority !== undefined &&
    typeof additionalReassertAuthority !== "function"
  ) {
    fail("mutation close additional authority was refused", 70);
  }
  const assertHeld = (publicationStage) => {
    const state = assertMutationLeaseHeld(roots, held, false);
    additionalReassertAuthority?.(state, publicationStage);
    return state;
  };
  assertHeld();
  const receiptAuthority =
    held.lease.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND
      ? backgroundReceiptReassertion(
          roots,
          assertHeld,
        )
      : held.lease.operation_kind ===
          CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
        ? backgroundCleanupReceiptReassertion(
            roots,
            assertHeld,
          )
        : assertHeld;
  reconcileReceiptPublication(roots, receiptAuthority);
  reconcileEnvironmentPublication(roots);
  const state = assertHeld();
  return publishMutationClose(
    roots,
    state,
    state.mutationLease,
    disposition,
    assertHeld,
    publicationHolds,
    operationEvidenceSha256,
  );
}

function withMutationLease(roots, action, callback) {
  const held = acquireMutationLease(roots, action);
  let result;
  let callbackError;
  try {
    result = callback();
  } catch (error) {
    callbackError = error;
  }
  assertMutationLeaseHeld(roots, held, false);
  reconcileReceiptPublication(roots);
  reconcileEnvironmentPublication(roots);
  const state = assertMutationLeaseHeld(roots, held, false);
  const receiptChanged =
    state.receiptState.head.sequence !== held.lease.source_sequence ||
    state.receiptState.head_sha256 !== held.lease.source_head_sha256;
  const environmentChanged =
    (state.environment === undefined ? ZERO_SHA256 : digest(state.environment.bytes)) !==
    held.lease.source_environment_sha256;
  if (
    callbackError !== undefined &&
    held.lease.action === "finalize-environment" &&
    environmentChanged &&
    !receiptChanged
  ) {
    throw callbackError;
  }
  closeMutationLease(
    roots,
    held,
    callbackError !== undefined && !receiptChanged && !environmentChanged
      ? "aborted-before-effect"
      : "completed",
  );
  if (callbackError !== undefined) throw callbackError;
  return result;
}

function validateFakeProviderAdapter(adapter) {
  exactKeys(
    adapter,
    FAKE_PROVIDER_ADAPTER_FIELDS,
    "fake provider adapter",
  );
  if (
    adapter.kind !== FAKE_PROVIDER_CONTRACT.kind ||
    !Number.isSafeInteger(adapter.close_prelink_hold_milliseconds) ||
    adapter.close_prelink_hold_milliseconds < 0 ||
    adapter.close_prelink_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.hold_milliseconds) ||
    adapter.hold_milliseconds < 0 ||
    adapter.hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.prelink_hold_milliseconds) ||
    adapter.prelink_hold_milliseconds < 0 ||
    adapter.prelink_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.publication_hold_milliseconds) ||
    adapter.publication_hold_milliseconds < 0 ||
    adapter.publication_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !Number.isSafeInteger(adapter.reconcile_hold_milliseconds) ||
    adapter.reconcile_hold_milliseconds < 0 ||
    adapter.reconcile_hold_milliseconds > FAKE_PROVIDER_CONTRACT.max_hold_milliseconds ||
    !new Set(["failed", "passed"]).has(adapter.execute_outcome) ||
    !new Set(["failed", "passed", "unknown"]).has(adapter.reconcile_outcome) ||
    adapter.execute_result === null ||
    Array.isArray(adapter.execute_result) ||
    typeof adapter.execute_result !== "object" ||
    adapter.reconcile_result === null ||
    Array.isArray(adapter.reconcile_result) ||
    typeof adapter.reconcile_result !== "object"
  ) {
    fail("fake provider adapter was refused", 64);
  }
}

function validateBackgroundProviderAdapter(adapter) {
  exactKeys(adapter, BACKGROUND_PROVIDER_ADAPTER_FIELDS, "background provider adapter");
  const outerHolds = [
    adapter.after_authority_hold_milliseconds,
    adapter.after_evidence_hold_milliseconds,
    adapter.after_intent_hold_milliseconds,
    adapter.after_result_hold_milliseconds,
    adapter.after_settlement_hold_milliseconds,
  ];
  const innerHolds = [
    adapter.before_detach_hold_milliseconds,
    adapter.before_identity_probe_hold_milliseconds,
    adapter.before_start_decision_hold_milliseconds,
    adapter.before_start_hold_milliseconds,
  ];
  if (
    adapter.kind !== "controlled-background-provider-v1" ||
    !Number.isSafeInteger(adapter.maximum_lifetime_milliseconds) ||
    adapter.maximum_lifetime_milliseconds < 1_000 ||
    adapter.maximum_lifetime_milliseconds > 30_000 ||
    outerHolds.some(
      (milliseconds) =>
        !Number.isSafeInteger(milliseconds) || milliseconds < 0 || milliseconds > 30_000,
    ) ||
    innerHolds.some(
      (milliseconds) =>
        !Number.isSafeInteger(milliseconds) || milliseconds < 0 || milliseconds > 5_000,
    )
  ) {
    fail("background provider adapter was refused", 64);
  }
}

function validateBackgroundCleanupAdapter(adapter) {
  exactKeys(
    adapter,
    BACKGROUND_CLEANUP_ADAPTER_FIELDS,
    "background cleanup adapter",
  );
  const holds = [
    adapter.after_claim_hold_milliseconds,
    adapter.after_intent_hold_milliseconds,
    adapter.after_plan_hold_milliseconds,
    adapter.after_result_hold_milliseconds,
    adapter.after_retirement_hold_milliseconds,
    adapter.after_settlement_hold_milliseconds,
    adapter.after_slot_hold_milliseconds,
    adapter.before_close_hold_milliseconds,
    adapter.close_prelink_hold_milliseconds,
  ];
  const optionalSequences = [
    adapter.crash_after_delete_sequence,
    adapter.crash_after_delete_syscall_sequence,
    adapter.stop_after_sequence,
  ];
  if (
    adapter.kind !== "controlled-background-provider-cleanup-v1" ||
    holds.some(
      (milliseconds) =>
        !Number.isSafeInteger(milliseconds) ||
        milliseconds < 0 ||
        milliseconds > 30_000,
    ) ||
    optionalSequences.some(
      (sequence) =>
        sequence !== null &&
        (!Number.isSafeInteger(sequence) || sequence < 0),
    ) ||
    (adapter.crash_after_delete_sequence !== null &&
      adapter.crash_after_delete_sequence < 1) ||
    (adapter.crash_after_delete_syscall_sequence !== null &&
      adapter.crash_after_delete_syscall_sequence < 1) ||
    typeof adapter.crash_after_hostagent_settlement !== "boolean"
  ) {
    fail("background cleanup adapter was refused", 64);
  }
}

function providerIntentResult(fixtureId) {
  return {
    operation_kind: DETERMINISTIC_PROVIDER_OPERATION_KIND,
    operation_plan_sha256: ZERO_SHA256,
    preexisting_resource: "absent",
    provider_contract_sha256: FAKE_PROVIDER_CONTRACT_SHA256,
    provider_resource: `synveda-cpr45-${fixtureId}`,
    provider_root_key: `sv-c45-${fixtureId.slice(0, 16)}`,
  };
}

function backgroundProviderIntentResult(slot) {
  const operationPlan = slot.value.operation_plan;
  return Object.freeze({
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(operationPlan),
    preexisting_resource: "absent",
    provider_contract_sha256: slot.value.operation_contract_sha256,
    provider_resource: operationPlan.provider_resource,
    provider_root_key: operationPlan.provider_root_key,
  });
}

function backgroundProviderPassedResult(slot, settlement) {
  return Object.freeze({
    evidence_class: "controlled-background-fake",
    operation_evidence_sha256: digest(settlement.bytes),
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(slot.value.operation_plan),
    platform: "deterministic-posix",
    provider_contract_sha256: slot.value.operation_contract_sha256,
    provider_name: "controlled-background-fake",
    runtime_name: "docker-fake",
  });
}

function backgroundProviderFailureResult(settlement, phase = "provider-create-failed") {
  if (phase === "execution-failed") return refusedProviderResult();
  if (settlement.value.safe_code === "resource-collision") {
    return Object.freeze({
      cleanup_required: phase !== "preflight-refused",
      collision_resource: "provider",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    });
  }
  return refusedProviderResult(settlement.value.safe_code);
}

function backgroundCleanupIntentResult(slot) {
  return Object.freeze({
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(slot.value.operation_plan),
    provider_resource: slot.value.operation_plan.provider_resource,
    retirement_contract_sha256: slot.value.operation_contract_sha256,
    scope: "exact-receipt-owned-only",
  });
}

function backgroundCleanupPassedResult(slot, settlement) {
  return Object.freeze({
    context_absent: true,
    inert_staging_absent: true,
    operation_evidence_sha256: digest(settlement.bytes),
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(slot.value.operation_plan),
    provider_absent: true,
    retirement_contract_sha256: slot.value.operation_contract_sha256,
    runtime_root_absent: true,
    socket_absent: true,
    source_closure_unchanged: true,
  });
}

function backgroundCleanupPrefixObservation(prefix, settlement) {
  const disposition =
    prefix.cleanupStage === "not-started"
      ? "not-reached"
      : prefix.cleanupStage === "settled"
        ? "complete"
        : "pending";
  return Object.freeze({
    observed_effect_disposition: disposition,
    observed_effect_name: "provider-cleanup",
    observed_evidence_head_sha256: prefix.createEvidenceHeadSha256,
    observed_evidence_prefix_sha256: prefix.observationSha256,
    observed_evidence_stage: prefix.cleanupStage,
    observed_residual_sha256: prefix.remainingInventorySha256,
    observed_settlement_sha256:
      settlement === undefined ? ZERO_SHA256 : digest(settlement.bytes),
  });
}

function validateBackgroundProspectiveReceipt(state, receipt) {
  const slot = state.mutationLease;
  if (
    slot?.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
    receipt.sequence !== state.receiptState.head.sequence + 1 ||
    receipt.previous_sha256 !== state.receiptState.head_sha256
  ) {
    fail("background provider receipt authority was refused", 73);
  }
  const settlement = state.operationSettlement;
  let expectedResult;
  if (receipt.phase === "provider-create-intent") {
    if (
      state.receiptState.head.sequence !== slot.value.source_sequence ||
      state.receiptState.head_sha256 !== slot.value.source_head_sha256 ||
      settlement !== undefined ||
      !backgroundPrefixIsEmpty(state.providerState.prefix)
    ) {
      fail("background provider intent authority was refused", 73);
    }
    expectedResult = backgroundProviderIntentResult(slot);
  } else if (receipt.phase === "preflight-refused") {
    if (
      state.receiptState.head.sequence !== slot.value.source_sequence ||
      state.receiptState.head_sha256 !== slot.value.source_head_sha256 ||
      settlement?.value.disposition !== "exact-residual" ||
      settlement.value.safe_code !== "resource-collision"
    ) {
      fail("background provider preflight authority was refused", 73);
    }
    expectedResult = backgroundProviderFailureResult(settlement, receipt.phase);
  } else if (receipt.phase === "provider-create-passed") {
    if (
      state.receiptState.head.phase !== "provider-create-intent" ||
      settlement?.value.disposition !== "complete-identity"
    ) {
      fail("background provider pass authority was refused", 73);
    }
    expectedResult = backgroundProviderPassedResult(slot, settlement);
  } else if (receipt.phase === "provider-create-failed") {
    if (
      state.receiptState.head.phase !== "provider-create-intent" ||
      settlement?.value.disposition !== "exact-residual"
    ) {
      fail("background provider failure authority was refused", 73);
    }
    expectedResult = backgroundProviderFailureResult(settlement);
  } else if (receipt.phase === "execution-failed") {
    if (
      !new Set(["provider-create-intent", "provider-create-passed"]).has(
        state.receiptState.head.phase,
      ) ||
      settlement?.value.disposition !== "complete-identity"
    ) {
      fail("background provider execution failure authority was refused", 73);
    }
    expectedResult = backgroundProviderFailureResult(settlement, receipt.phase);
  } else {
    fail("background provider receipt phase was refused", 73);
  }
  if (canonical(receipt.result) !== canonical(expectedResult)) {
    fail("background provider receipt result was refused", 73);
  }
}

function backgroundReceiptReassertion(roots, assertAuthority) {
  if (typeof assertAuthority !== "function") {
    fail("background provider receipt authority was refused", 70);
  }
  return (receipt) => {
    const state = assertAuthority();
    validateBackgroundProspectiveReceipt(state, receipt);
    if (
      new Set(["provider-create-intent", "provider-create-passed"]).has(receipt.phase) &&
      !sourceClosureMatches(roots, state.candidate)
    ) {
      fail("background provider source closure changed", 73);
    }
  };
}

function providerAdapterReceipt(adapterResult) {
  if (
    adapterResult === null ||
    typeof adapterResult !== "object" ||
    adapterResult.then !== undefined ||
    !new Set(["failed", "passed"]).has(adapterResult.outcome) ||
    adapterResult.result === null ||
    Array.isArray(adapterResult.result) ||
    typeof adapterResult.result !== "object"
  ) {
    fail("fake provider result was refused", 70);
  }
  if (
    adapterResult.outcome === "passed" &&
    (adapterResult.result.evidence_class !== "deterministic-fixture" ||
      adapterResult.result.provider_contract_sha256 !== FAKE_PROVIDER_CONTRACT_SHA256)
  ) {
    fail("fake provider result was refused", 70);
  }
  return {
    phase: adapterResult.outcome === "passed" ? "provider-create-passed" : "provider-create-failed",
    result: adapterResult.result,
  };
}

function holdFakeProvider(milliseconds) {
  if (milliseconds === 0) return;
  const cell = new Int32Array(new SharedArrayBuffer(4));
  Atomics.wait(cell, 0, 0, milliseconds);
}

function refusedProviderResult(safeCode = "evidence-refused") {
  return {
    cleanup_required: true,
    collision_resource: "none",
    resource_disposition: "receipt-owned-or-absent",
    safe_code: safeCode,
  };
}

function validateLiveProviderOperationPlanForState(value) {
  try {
    return validateColimaLiveProviderOperationPlan(value);
  } catch (error) {
    if (error instanceof LiveProviderPlanFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function liveProviderPlanStateBinding(state) {
  return Object.freeze({
    candidate_sha256: digest(state.candidateBytes),
    fixture_id: state.candidate.run_id,
    source_head_sha256: state.receiptState.head_sha256,
    source_sequence: state.receiptState.head.sequence,
  });
}

function assertLiveProviderOperationPlanStateBinding(operationPlan, state) {
  const expected = liveProviderPlanStateBinding(state);
  if (
    operationPlan.fixture_id !== expected.fixture_id ||
    operationPlan.source_candidate_sha256 !== expected.candidate_sha256 ||
    operationPlan.source_head_sha256 !== expected.source_head_sha256 ||
    operationPlan.source_sequence !== expected.source_sequence ||
    operationPlan.provider_resource !== state.plan.result.provider_resource
  ) {
    fail("live provider planning operation binding was refused", 73);
  }
  return operationPlan;
}

function assertLiveProviderPlanningState(
  roots,
  state,
  { allowClosePublicationStage = false } = {},
) {
  const stateEntries = readdirSync(roots.stateBase).sort();
  const expectedEntries = [`.run-${state.candidate.run_id}`, "active"].sort();
  const nonPlanSlots = state.mutationSlots.filter(
    (slot) => slot.value.action !== LIVE_PROVIDER_PLAN_ACTION,
  );
  const closedSequences = new Set(
    state.mutationCloses.map((close) => close.value.slot_sequence),
  );
  const abandonedHistoricalPlan = state.mutationSlots.some(
    (slot) =>
      slot !== state.mutationLease &&
      slot !== state.liveProviderPlan?.slot &&
      (!closedSequences.has(slot.value.journal_sequence) ||
        state.mutationCloses.find(
          (close) => close.value.slot_sequence === slot.value.journal_sequence,
        )?.value.disposition !== "aborted-before-effect"),
  );
  if (
    JSON.stringify(stateEntries) !== JSON.stringify(expectedEntries) ||
    state.receipts.length !== 1 ||
    state.receiptState.head.phase !== "plan" ||
    state.receiptState.head.sequence !== 0 ||
    state.pendingPublication !== undefined ||
    state.environment !== undefined ||
    state.environmentPublication !== undefined ||
    (allowClosePublicationStage
      ? state.mutationStages.length > 1
      : state.mutationStages.length !== 0) ||
    state.mutationOperations.length !== 0 ||
    nonPlanSlots.length !== 0 ||
    abandonedHistoricalPlan ||
    !new Set(["live-provider-plan-only", "synchronous-fake"]).has(
      state.providerState.contract,
    ) ||
    state.cleanupState.contract !== "journal-only"
  ) {
    fail("live provider planning requires pristine state", 73);
  }
  return state;
}

function refuseCompletedLiveProviderPlan(state) {
  if (state.liveProviderPlan !== undefined) {
    fail("live provider execution remains disabled after state planning", 73);
  }
}

function buildLiveProviderOperationPlan(argumentsValue, state) {
  try {
    return buildColimaLiveProviderOperationPlan({
      observation: argumentsValue.observation,
      observationInput: argumentsValue.observationInput,
      stateBinding: liveProviderPlanStateBinding(state),
      tuple: argumentsValue.adapterTuple,
    });
  } catch (error) {
    if (error instanceof LiveProviderPlanFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function recordLiveProviderOperationPlan(roots, operationPlan, rebuild) {
  validateLiveProviderOperationPlanForState(operationPlan);
  const initial = assertLiveProviderPlanningState(
    roots,
    loadState(roots, true),
  );
  assertLiveProviderOperationPlanStateBinding(operationPlan, initial);
  if (initial.liveProviderPlan !== undefined) {
    if (
      !liveProviderPlanBytes(initial.liveProviderPlan.operationPlan).equals(
        liveProviderPlanBytes(operationPlan),
      )
    ) {
      fail("live provider operation plan already differs", 73);
    }
    return initial.liveProviderPlan;
  }
  if (initial.mutationLease !== undefined) {
    fail("an abandoned live provider plan requires explicit recovery", 73);
  }
  const expectedSource = Object.freeze({
    sequence: initial.receiptState.head.sequence,
    sha256: initial.receiptState.head_sha256,
  });
  const held = acquireMutationLease(
    roots,
    LIVE_PROVIDER_PLAN_ACTION,
    ZERO_SHA256,
    expectedSource,
    {},
    Object.freeze({
      contractSha256: operationPlan.operation_contract_sha256,
      kind: operationPlan.operation_kind,
      plan: operationPlan,
    }),
  );
  let planningFailure;
  const reassertPlanningAuthority = () => {
    assertMutationLeaseHeld(roots, held, false);
    const current = assertLiveProviderPlanningState(
      roots,
      loadState(roots, true),
      { allowClosePublicationStage: true },
    );
    if (
      current.mutationLease === undefined ||
      !current.mutationLease.bytes.equals(held.leaseBytes)
    ) {
      fail("live provider planning authority changed", 73);
    }
    const rebuilt = rebuild(current);
    validateLiveProviderOperationPlanForState(rebuilt);
    if (
      !liveProviderPlanBytes(rebuilt).equals(liveProviderPlanBytes(operationPlan))
    ) {
      fail("live provider preparation changed during state planning", 73);
    }
  };
  try {
    reassertPlanningAuthority();
  } catch (error) {
    planningFailure = error;
  }
  closeMutationLease(
    roots,
    held,
    planningFailure === undefined ? "completed" : "aborted-before-effect",
    {},
    ZERO_SHA256,
    planningFailure === undefined ? reassertPlanningAuthority : undefined,
  );
  if (planningFailure !== undefined) throw planningFailure;
  const verified = loadState(roots, true);
  if (
    verified.liveProviderPlan === undefined ||
    !liveProviderPlanBytes(verified.liveProviderPlan.operationPlan).equals(
      liveProviderPlanBytes(operationPlan),
    )
  ) {
    fail("live provider operation plan was not durable", 70);
  }
  return verified.liveProviderPlan;
}

export function recordLiveProviderOperationPlanForExecutor(argumentsValue) {
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  reconcileMutationStages(roots);
  const initial = assertLiveProviderPlanningState(roots, loadState(roots, true));
  const operationPlan = buildLiveProviderOperationPlan(argumentsValue, initial);
  return recordLiveProviderOperationPlan(
    roots,
    operationPlan,
    (current) => buildLiveProviderOperationPlan(argumentsValue, current),
  );
}

// This owner-UID test seam exercises journal serialization with a prebuilt
// production-shaped plan. The supported lifecycle never imports or exposes it.
export function recordLiveProviderOperationPlanForTest(argumentsValue) {
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  reconcileMutationStages(roots);
  const operationPlan = validateLiveProviderOperationPlanForState(
    argumentsValue.operationPlan,
  );
  const holdMilliseconds = argumentsValue.holdMilliseconds ?? 0;
  if (
    !Number.isSafeInteger(holdMilliseconds) ||
    holdMilliseconds < 0 ||
    holdMilliseconds > 30_000
  ) {
    fail("live provider plan test hold was refused", 64);
  }
  return recordLiveProviderOperationPlan(roots, operationPlan, () => {
    holdFakeProvider(holdMilliseconds);
    return operationPlan;
  });
}

function completedLiveProviderPlanCoreSnapshot(state) {
  const completed = state.liveProviderPlan;
  if (
    completed === undefined ||
    state.pendingPublication !== undefined ||
    state.environment !== undefined ||
    state.environmentPublication !== undefined ||
    state.mutationOperations.length !== 0 ||
    state.receipts.length !== 1 ||
    state.receiptState.head.phase !== "plan" ||
    state.receiptState.head.sequence !== 0 ||
    !new Set([
      "live-provider-fixture-start-decision-only",
      "live-provider-fixture-intent-only",
      "live-provider-start-decision-only",
      "live-provider-intent-only",
      "live-provider-plan-only",
    ]).has(state.providerState.contract) ||
    state.providerState.operationEvidenceSha256 !== ZERO_SHA256 ||
    completed.close.value.disposition !== "completed" ||
    completed.close.value.authority !== "owner" ||
    completed.close.value.operation_evidence_sha256 !== ZERO_SHA256 ||
    completed.close.value.result_environment_sha256 !== ZERO_SHA256
  ) {
    fail("completed live provider plan was unavailable", 69);
  }
  const operationPlan = completed.operationPlan;
  return Object.freeze({
    completionProjection: buildColimaLivePlanCompletionProjectionStructure({
      operationPlanSha256: liveProviderPlanDigest(
        liveProviderPlanBytes(operationPlan),
      ),
      planCloseSha256: digest(completed.close.bytes),
      planSlotSha256: digest(completed.slot.bytes),
      preparationObservationSha256:
        operationPlan.preparation_observation_sha256,
    }),
    operationPlan,
  });
}

function abortedLiveProviderIntentTail(state) {
  const completedPlanSequence = state.liveProviderPlan.slot.value.journal_sequence;
  const tail = state.mutationSlots.slice(completedPlanSequence + 1);
  const closes = new Map(
    state.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  if (
    tail.some(
      (slot) =>
        slot.value.action !== COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
        closes.get(slot.value.journal_sequence)?.value.disposition !==
          "aborted-before-effect",
    )
  ) {
    fail("completed live provider plan was unavailable", 69);
  }
  return tail;
}

function completedLiveProviderPlanSnapshot(state) {
  const snapshot = completedLiveProviderPlanCoreSnapshot(state);
  const tail = abortedLiveProviderIntentTail(state);
  const expectedLastClose =
    tail.length === 0
      ? state.liveProviderPlan.close
      : state.mutationCloses.find(
          (close) =>
            close.value.slot_sequence === tail.at(-1).value.journal_sequence,
        );
  if (
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.mutationStages.length !== 0 ||
    state.liveProviderIntent !== undefined ||
    state.liveProviderFixtureIntent !== undefined ||
    state.providerState.contract !== "live-provider-plan-only" ||
    state.mutationCloses.at(-1) !== expectedLastClose
  ) {
    fail("completed live provider plan was unavailable", 69);
  }
  return snapshot;
}

function validateLiveProviderIntentPublicationStages(state, ownStage) {
  if (
    state.mutationStages.some(
      (stage) => stage.linkedDestination !== undefined,
    )
  ) {
    fail("live provider intent publication stage was refused", 73);
  }
  if (ownStage === undefined) return;
  const staged = state.mutationStages.find(
    (stage) => stage.path === ownStage.stagePath,
  );
  if (
    staged === undefined ||
    staged.name !== ownStage.stagePath.split(sep).at(-1) ||
    !sameMetadata(staged.metadata, ownStage.identity)
  ) {
    fail("live provider intent publication stage changed", 73);
  }
  const parsed = parseCanonical(
    ownStage.stagePath,
    "live provider intent publication stage",
    1,
  );
  if (!parsed.bytes.equals(ownStage.bytes)) {
    fail("live provider intent publication stage changed", 73);
  }
}

function availableLiveProviderIntentPlanSnapshot(
  state,
  publicationPlan,
  ownStage,
  fixtureOnly,
) {
  const snapshot = completedLiveProviderPlanCoreSnapshot(state);
  assertLiveProviderIntentVariantHistory(state, fixtureOnly);
  const tail = abortedLiveProviderIntentTail(state);
  const expectedLastClose =
    tail.length === 0
      ? state.liveProviderPlan.close
      : state.mutationCloses.find(
          (close) =>
            close.value.slot_sequence === tail.at(-1).value.journal_sequence,
        );
  if (
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.liveProviderIntent !== undefined ||
    state.liveProviderFixtureIntent !== undefined ||
    state.providerState.contract !== "live-provider-plan-only" ||
    state.mutationCloses.at(-1) !== expectedLastClose
  ) {
    fail("live provider intent publication state was refused", 73);
  }
  validateLiveProviderIntentPublicationStages(state, ownStage);
  if (ownStage !== undefined) {
    const staged = parseCanonical(
      ownStage.stagePath,
      "prospective live provider intent slot",
      1,
    );
    validateMutationLeaseValue(staged.value, state.candidate.run_id);
    const previousClose = state.mutationCloses.at(-1);
    if (
      ownStage.destinationName !==
        mutationSlotFileName(state.mutationSlots.length) ||
      staged.value.action !== COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
      staged.value.journal_sequence !== state.mutationSlots.length ||
      staged.value.source_sequence !== 0 ||
      staged.value.source_head_sha256 !== state.receiptState.head_sha256 ||
      staged.value.source_environment_sha256 !== ZERO_SHA256 ||
      staged.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      staged.value.previous_close_sha256 !== digest(previousClose.bytes) ||
      !canonicalBytes(staged.value.operation_plan).equals(
        canonicalBytes(publicationPlan),
      ) ||
      mutationOwnerState(staged.value) !== "current"
    ) {
      fail("prospective live provider intent slot was refused", 73);
    }
  }
  return snapshot;
}

function heldLiveProviderIntentPlanSnapshot(
  state,
  held,
  publicationPlan,
  ownStage,
  fixtureOnly,
) {
  const snapshot = completedLiveProviderPlanCoreSnapshot(state);
  assertLiveProviderIntentVariantHistory(state, fixtureOnly);
  const planSequence = state.liveProviderPlan.slot.value.journal_sequence;
  const tail = state.mutationSlots.slice(planSequence + 1);
  const intentSlot = tail.at(-1);
  const closes = new Map(
    state.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  const current = parseCanonical(held.path, "mutation slot");
  const currentIdentity = mutationHeldIdentity(held.path);
  if (
    intentSlot === undefined ||
    intentSlot !== state.mutationLease ||
    !intentSlot.bytes.equals(held.leaseBytes) ||
    !current.bytes.equals(held.leaseBytes) ||
    !sameMetadata(currentIdentity, held.identity) ||
    intentSlot.value.action !== COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
    !canonicalBytes(intentSlot.value.operation_plan).equals(
      canonicalBytes(publicationPlan),
    ) ||
    tail
      .slice(0, -1)
      .some(
        (slot) =>
          slot.value.action !== COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
          closes.get(slot.value.journal_sequence)?.value.disposition !==
            "aborted-before-effect",
      ) ||
    state.mutationRecoveries.length !== 0 ||
    state.liveProviderIntent !== undefined ||
    state.liveProviderFixtureIntent !== undefined ||
    state.providerState.contract !== "live-provider-plan-only"
  ) {
    fail("held live provider intent state was refused", 73);
  }
  validateLiveProviderIntentPublicationStages(state, ownStage);
  if (ownStage !== undefined) {
    const stagedClose = parseCanonical(
      ownStage.stagePath,
      "prospective live provider intent close",
      1,
    );
    validateMutationCloseValue(
      stagedClose.value,
      intentSlot,
      state.candidate.run_id,
    );
    if (
      ownStage.destinationName !==
        mutationCloseFileName(intentSlot.value.journal_sequence) ||
      stagedClose.value.disposition !== "completed" ||
      stagedClose.value.authority !== "owner" ||
      stagedClose.value.result_sequence !== 0 ||
      stagedClose.value.result_head_sha256 !==
        state.receiptState.head_sha256 ||
      stagedClose.value.result_environment_sha256 !== ZERO_SHA256 ||
      stagedClose.value.operation_evidence_sha256 !== ZERO_SHA256
    ) {
      fail("prospective live provider intent close was refused", 73);
    }
  }
  return snapshot;
}

function liveProviderIntentCompletion(completed) {
  return buildColimaLiveProviderIntentCompletion({
    closeSha256: digest(completed.close.bytes),
    fixtureOnly: completed.fixtureOnly,
    publicationPlan: completed.publicationPlan,
    slotSha256: digest(completed.slot.bytes),
  });
}

function abortedLiveProviderStartDecisionTail(state, completedIntent) {
  const intentSequence = completedIntent.slot.value.journal_sequence;
  const tail = state.mutationSlots.slice(intentSequence + 1);
  const closes = new Map(
    state.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  if (
    tail.some(
      (slot) =>
        slot.value.action !== COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
        closes.get(slot.value.journal_sequence)?.value.disposition !==
          "aborted-before-effect",
    )
  ) {
    fail("completed live provider intent was unavailable", 69);
  }
  return tail;
}

function completedLiveProviderIntentSourceSnapshot(
  state,
  fixtureOnly,
  providerStatePhase,
) {
  if (!new Set(["intent", "start-decision"]).has(providerStatePhase)) {
    fail("completed live provider intent snapshot phase was refused", 70);
  }
  const planSnapshot = completedLiveProviderPlanCoreSnapshot(state);
  const completed = completedLiveProviderIntentForVariant(state, fixtureOnly);
  assertLiveProviderIntentVariantHistory(state, fixtureOnly);
  const expectedProviderContract = `live-provider-${
    fixtureOnly ? "fixture-" : ""
  }${providerStatePhase}-only`;
  if (
    completed === undefined ||
    completed.fixtureOnly !== fixtureOnly ||
    completed.close.value.slot_sequence !==
      completed.slot.value.journal_sequence ||
    completed.close.value.disposition !== "completed" ||
    completed.close.value.authority !== "owner" ||
    completed.close.value.result_sequence !== 0 ||
    completed.close.value.result_head_sha256 !==
      state.receiptState.head_sha256 ||
    completed.close.value.result_environment_sha256 !== ZERO_SHA256 ||
    completed.close.value.operation_evidence_sha256 !== ZERO_SHA256 ||
    state.pendingPublication !== undefined ||
    state.environment !== undefined ||
    state.environmentPublication !== undefined ||
    state.operationSettlement !== undefined ||
    state.cleanupSettlement !== undefined ||
    state.providerState.contract !== expectedProviderContract ||
    state.providerState.operationEvidenceSha256 !== ZERO_SHA256 ||
    state.cleanupState.contract !== "journal-only" ||
    state.cleanupState.operationEvidenceSha256 !== ZERO_SHA256
  ) {
    fail("completed live provider intent was unavailable", 69);
  }

  const source = Object.freeze({
    closeAuthority: completed.close.value.authority,
    fixtureOnly: completed.fixtureOnly,
    intentCompletion: liveProviderIntentCompletion(completed),
    intentPublicationPlan: completed.publicationPlan,
  });
  let completedIntentProjection;
  try {
    completedIntentProjection =
      buildColimaLiveCompletedProviderIntentProjectionStructure(source);
  } catch (error) {
    if (error instanceof LiveProviderStartDecisionFailure) {
      fail("completed live provider intent projection was refused", error.exitStatus);
    }
    throw error;
  }
  return Object.freeze({
    ...planSnapshot,
    completedIntentProjection,
    source,
  });
}

function completedLiveProviderIntentCoreSnapshot(state, fixtureOnly) {
  return completedLiveProviderIntentSourceSnapshot(
    state,
    fixtureOnly,
    "intent",
  );
}

function completedLiveProviderIntentSnapshot(state, fixtureOnly) {
  const snapshot = completedLiveProviderIntentCoreSnapshot(state, fixtureOnly);
  const completed = completedLiveProviderIntentForVariant(state, fixtureOnly);
  const decisionTail = abortedLiveProviderStartDecisionTail(state, completed);
  const expectedLastClose =
    decisionTail.length === 0
      ? completed.close
      : state.mutationCloses.find(
          (close) =>
            close.value.slot_sequence ===
            decisionTail.at(-1).value.journal_sequence,
        );
  if (
    state.mutationCloses.at(-1) !== expectedLastClose ||
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.mutationStages.length !== 0
  ) {
    fail("completed live provider intent was unavailable", 69);
  }
  return snapshot;
}

function admissionArguments(argumentsValue, additionalFields = []) {
  const fields = [
    "observation",
    "observationInput",
    "repoRoot",
    "stateBase",
    ...additionalFields,
  ];
  if (
    argumentsValue === null ||
    Array.isArray(argumentsValue) ||
    typeof argumentsValue !== "object" ||
    JSON.stringify(Object.keys(argumentsValue).sort()) !==
      JSON.stringify(fields.sort()) ||
    typeof argumentsValue.repoRoot !== "string" ||
    argumentsValue.repoRoot.length === 0 ||
    typeof argumentsValue.stateBase !== "string" ||
    argumentsValue.stateBase.length === 0 ||
    argumentsValue.observation === null ||
    Array.isArray(argumentsValue.observation) ||
    typeof argumentsValue.observation !== "object" ||
    argumentsValue.observationInput === null ||
    Array.isArray(argumentsValue.observationInput) ||
    typeof argumentsValue.observationInput !== "object"
  ) {
    fail("live provider pre-effect admission arguments were refused", 64);
  }
  return argumentsValue;
}

function observeLivePreEffectRoots(argumentsValue, snapshot, fixtureOnly) {
  try {
    if (
      colimaLiveDigest(colimaLiveBytes(argumentsValue.observation)) !==
      snapshot.operationPlan.preparation_observation_sha256
    ) {
      fail("live provider pre-effect observation binding was refused", 73);
    }
    return fixtureOnly
      ? observeColimaLivePreEffectRootsForTest(
          argumentsValue.requirements,
          argumentsValue.observation,
          argumentsValue.observationInput,
        )
      : observeColimaLivePreEffectRoots(
          argumentsValue.observation,
          argumentsValue.observationInput,
        );
  } catch (error) {
    if (error instanceof ColimaLiveContractFailure) {
      fail("live provider pre-effect observation was refused", error.exitStatus);
    }
    throw error;
  }
}

function sameLiveProviderPlanSnapshot(left, right) {
  return (
    canonicalBytes(left.completionProjection).equals(
      canonicalBytes(right.completionProjection),
    ) &&
    liveProviderPlanBytes(left.operationPlan).equals(
      liveProviderPlanBytes(right.operationPlan),
    )
  );
}

function sameCompletedLiveProviderIntentSnapshot(left, right) {
  return (
    sameLiveProviderPlanSnapshot(left, right) &&
    left.source.closeAuthority === right.source.closeAuthority &&
    left.source.fixtureOnly === right.source.fixtureOnly &&
    liveProviderIntentBytes(left.source.intentCompletion).equals(
      liveProviderIntentBytes(right.source.intentCompletion),
    ) &&
    liveProviderIntentBytes(left.source.intentPublicationPlan).equals(
      liveProviderIntentBytes(right.source.intentPublicationPlan),
    ) &&
    liveProviderStartDecisionBytes(left.completedIntentProjection).equals(
      liveProviderStartDecisionBytes(right.completedIntentProjection),
    )
  );
}

function validateLivePreEffectRootBinding(rootObservation, snapshot, fixtureOnly) {
  const operationPlan = snapshot.operationPlan;
  exactKeys(
    rootObservation,
    [
      "evidence_class",
      "planned_names",
      "preparation_observation_sha256",
      "requirements_sha256",
      "root_observations",
      "root_set_disposition",
      "schema",
    ],
    "live provider pre-effect root observation",
  );
  exactKeys(
    rootObservation.planned_names,
    ["lima_instance", "provider_profile"],
    "live provider pre-effect planned names",
  );
  const expectedRoles = COLIMA_LIVE_MUTATION_SURFACE_ROLES;
  if (
    !Array.isArray(rootObservation.root_observations) ||
    rootObservation.root_observations.length !== expectedRoles.length
  ) {
    fail("live provider pre-effect root observation was refused", 70);
  }
  for (const [index, root] of rootObservation.root_observations.entries()) {
    exactKeys(
      root,
      [
        "baseline_descriptor_set_hmac_sha256",
        "baseline_descriptors",
        "baseline_relative_identity_hmac_sha256",
        "disposition",
        "namespace_identity_hmac_sha256",
        "observed_entry_set_hmac_sha256",
        "role",
      ],
      "live provider pre-effect root",
    );
    if (
      !Array.isArray(root.baseline_relative_identity_hmac_sha256) ||
      !Array.isArray(root.baseline_descriptors) ||
      root.baseline_descriptors.length !==
        root.baseline_relative_identity_hmac_sha256.length ||
      root.baseline_descriptors.length >
        COLIMA_LIVE_MAX_BASELINE_DESCENDANTS
    ) {
      fail("live provider pre-effect baseline descriptors were refused", 70);
    }
    for (const descriptor of root.baseline_descriptors) {
      exactKeys(
        descriptor,
        COLIMA_LIVE_BASELINE_DESCRIPTOR_BINDING_FIELDS,
        "live provider pre-effect baseline descriptor",
      );
      if (
        !onlyLowerHex(descriptor.descriptor_sha256, 64) ||
        descriptor.descriptor_sha256 === ZERO_SHA256 ||
        !onlyLowerHex(descriptor.relative_identity_hmac_sha256, 64) ||
        descriptor.relative_identity_hmac_sha256 === ZERO_SHA256
      ) {
        fail("live provider pre-effect baseline descriptor was refused", 70);
      }
    }
    if (
      new Set(
        root.baseline_descriptors.map((entry) => entry.descriptor_sha256),
      ).size !== root.baseline_descriptors.length ||
      canonical(
        root.baseline_descriptors.map(
          (entry) => entry.relative_identity_hmac_sha256,
        ),
      ) !== canonical(root.baseline_relative_identity_hmac_sha256)
    ) {
      fail("live provider pre-effect baseline descriptors were refused", 70);
    }
    if (
      root.role !== expectedRoles[index] ||
      !new Set(["foreign-collision", "observed-pristine"]).has(
        root.disposition,
      ) ||
      !onlyLowerHex(root.baseline_descriptor_set_hmac_sha256, 64) ||
      root.baseline_descriptor_set_hmac_sha256 === ZERO_SHA256 ||
      !Array.isArray(root.baseline_relative_identity_hmac_sha256) ||
      root.baseline_relative_identity_hmac_sha256.length >
        COLIMA_LIVE_MAX_BASELINE_DESCENDANTS ||
      root.baseline_relative_identity_hmac_sha256.some(
        (identity) =>
          !onlyLowerHex(identity, 64) || identity === ZERO_SHA256,
      ) ||
      new Set(root.baseline_relative_identity_hmac_sha256).size !==
        root.baseline_relative_identity_hmac_sha256.length ||
      canonical(root.baseline_relative_identity_hmac_sha256) !==
        canonical([...root.baseline_relative_identity_hmac_sha256].sort()) ||
      root.baseline_relative_identity_hmac_sha256.includes(
        root.namespace_identity_hmac_sha256,
      ) ||
      !onlyLowerHex(root.namespace_identity_hmac_sha256, 64) ||
      root.namespace_identity_hmac_sha256 === ZERO_SHA256 ||
      !onlyLowerHex(root.observed_entry_set_hmac_sha256, 64) ||
      root.observed_entry_set_hmac_sha256 === ZERO_SHA256
    ) {
      fail("live provider pre-effect root observation was refused", 70);
    }
  }
  const baselineIdentities = rootObservation.root_observations.flatMap(
    (root) => root.baseline_relative_identity_hmac_sha256,
  );
  const namespaceIdentities = new Set(
    rootObservation.root_observations.map(
      (root) => root.namespace_identity_hmac_sha256,
    ),
  );
  if (
    baselineIdentities.length > COLIMA_LIVE_MAX_BASELINE_DESCENDANTS ||
    new Set(baselineIdentities).size !== baselineIdentities.length ||
    namespaceIdentities.size !== rootObservation.root_observations.length ||
    baselineIdentities.some((identity) => namespaceIdentities.has(identity))
  ) {
    fail("live provider pre-effect baseline identity set was refused", 70);
  }
  const expectedDisposition = rootObservation.root_observations.some(
    (root) => root.disposition === "foreign-collision",
  )
    ? "foreign-collision"
    : "observed-pristine";
  if (
    rootObservation.evidence_class !==
      (fixtureOnly ? "fixture-only" : "production-pinned") ||
    rootObservation.schema !==
      (fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA
        : COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA) ||
    (!fixtureOnly &&
      rootObservation.requirements_sha256 !==
        operationPlan.requirements_sha256) ||
    !onlyLowerHex(rootObservation.requirements_sha256, 64) ||
    rootObservation.root_set_disposition !== expectedDisposition ||
    rootObservation.preparation_observation_sha256 !==
      operationPlan.preparation_observation_sha256 ||
    rootObservation.planned_names.provider_profile !==
      operationPlan.provider_profile ||
    rootObservation.planned_names.lima_instance !==
      `colima-${operationPlan.provider_profile}` ||
    operationPlan.provider_resource !== operationPlan.provider_profile
  ) {
    fail("live provider pre-effect observation binding was refused", 73);
  }
}

function deepFreezeAdmission(value) {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) deepFreezeAdmission(child);
    Object.freeze(value);
  }
  return value;
}

function liveSupervisorProcessGroupLabel(snapshot) {
  const label =
    `sv-c45-colima-pg-${snapshot.operationPlan.fixture_id}-` +
    snapshot.completionProjection.plan_slot_sha256.slice(0, 12);
  if (
    !/^sv-c45-colima-pg-[0-9a-f]{32}-[0-9a-f]{12}$/u.test(label) ||
    Buffer.byteLength(label, "ascii") > 63
  ) {
    fail("live provider supervisor label was refused", 70);
  }
  return label;
}

export function projectLiveProviderPlanCompletionForExecutor(argumentsValue) {
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  return completedLiveProviderPlanSnapshot(
    loadState(roots, true),
  ).completionProjection;
}

function observeColimaLivePreEffectAdmission(
  admittedArguments,
  {
    admissionSchema,
    authority,
    fixtureOnly,
    stateSnapshot = completedLiveProviderPlanSnapshot,
    testCheckpoint,
  },
) {
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const firstState = stateSnapshot(loadState(roots, true));
  const firstRoots = observeLivePreEffectRoots(
    admittedArguments,
    firstState,
    fixtureOnly,
  );
  validateLivePreEffectRootBinding(firstRoots, firstState, fixtureOnly);
  if (testCheckpoint !== undefined) {
    testCheckpoint("after-first-admission-observation");
  }

  const secondState = stateSnapshot(loadState(roots, true));
  if (!sameLiveProviderPlanSnapshot(firstState, secondState)) {
    fail("live provider pre-effect state changed", 73);
  }
  const secondRoots = observeLivePreEffectRoots(
    admittedArguments,
    secondState,
    fixtureOnly,
  );
  validateLivePreEffectRootBinding(secondRoots, secondState, fixtureOnly);
  if (!colimaLiveBytes(firstRoots).equals(colimaLiveBytes(secondRoots))) {
    fail("live provider pre-effect observation changed", 73);
  }

  let intentCandidate = null;
  let preEffectPrefix = null;
  if (secondRoots.root_set_disposition === "observed-pristine") {
    intentCandidate = buildColimaLiveEffectIntentCandidateStructure({
      completionProjection: secondState.completionProjection,
      operationPlan: secondState.operationPlan,
    });
    preEffectPrefix = buildColimaLiveEmptyPreEffectPrefixStructure({
      completionProjection: secondState.completionProjection,
      intent: intentCandidate,
      operationPlan: secondState.operationPlan,
    });
  }
  return deepFreezeAdmission({
    authority,
    completion_projection: secondState.completionProjection,
    intent_candidate: intentCandidate,
    pre_effect_prefix: preEffectPrefix,
    root_observation: secondRoots,
    schema: admissionSchema,
    supervisor_process_group_label:
      liveSupervisorProcessGroupLabel(secondState),
  });
}

export function observeColimaLivePreEffectAdmissionForExecutor(argumentsValue) {
  return observeColimaLivePreEffectAdmission(
    admissionArguments(argumentsValue),
    {
      admissionSchema: COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
      authority: "point-in-time-not-effect-authority",
      fixtureOnly: false,
      testCheckpoint: undefined,
    },
  );
}

// This unsupported fixture seam substitutes only the small-file observation
// requirements. State, plan and projection provenance remain internally read;
// the caller cannot supply any of them, and the result has a distinct schema.
export function observeColimaLivePreEffectAdmissionForTest(argumentsValue) {
  const admittedArguments = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admittedArguments.requirements === null ||
    Array.isArray(admittedArguments.requirements) ||
    typeof admittedArguments.requirements !== "object" ||
    typeof admittedArguments.testCheckpoint !== "function"
  ) {
    fail("live provider fixture admission arguments were refused", 64);
  }
  return observeColimaLivePreEffectAdmission(
    admittedArguments,
    {
      admissionSchema: COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
      authority: "fixture-only-point-in-time-not-effect-authority",
      fixtureOnly: true,
      testCheckpoint: admittedArguments.testCheckpoint,
    },
  );
}

function buildLiveProviderStartFreshAdmission(snapshot, rootObservation) {
  try {
    return buildColimaLiveProviderStartFreshAdmissionStructure({
      completedIntentProjection: snapshot.completedIntentProjection,
      rootObservation,
      source: snapshot.source,
    });
  } catch (error) {
    if (error instanceof LiveProviderStartDecisionFailure) {
      fail("live provider start fresh admission was refused", error.exitStatus);
    }
    throw error;
  }
}

function observeColimaLiveProviderStartFreshAdmission(
  admittedArguments,
  {
    fixtureOnly,
    stateSnapshot = (state) =>
      completedLiveProviderIntentSnapshot(state, fixtureOnly),
    testCheckpoint,
  },
) {
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const firstState = stateSnapshot(loadState(roots, true));
  const firstRoots = observeLivePreEffectRoots(
    admittedArguments,
    firstState,
    fixtureOnly,
  );
  validateLivePreEffectRootBinding(firstRoots, firstState, fixtureOnly);
  const firstAdmission = buildLiveProviderStartFreshAdmission(
    firstState,
    firstRoots,
  );
  testCheckpoint?.("after-first-process-start-admission-observation");

  const secondState = stateSnapshot(loadState(roots, true));
  if (!sameCompletedLiveProviderIntentSnapshot(firstState, secondState)) {
    fail("completed live provider intent state changed", 73);
  }
  const secondRoots = observeLivePreEffectRoots(
    admittedArguments,
    secondState,
    fixtureOnly,
  );
  validateLivePreEffectRootBinding(secondRoots, secondState, fixtureOnly);
  if (!colimaLiveBytes(firstRoots).equals(colimaLiveBytes(secondRoots))) {
    fail("live provider start fresh observation changed", 73);
  }
  const secondAdmission = buildLiveProviderStartFreshAdmission(
    secondState,
    secondRoots,
  );
  if (
    !liveProviderStartDecisionBytes(firstAdmission).equals(
      liveProviderStartDecisionBytes(secondAdmission),
    )
  ) {
    fail("live provider start fresh admission changed", 73);
  }
  return secondAdmission;
}

export function observeColimaLiveProviderStartFreshAdmissionForExecutor(
  argumentsValue,
) {
  return observeColimaLiveProviderStartFreshAdmission(
    admissionArguments(argumentsValue),
    { fixtureOnly: false, testCheckpoint: undefined },
  );
}

// This unsupported fixture seam substitutes only the small-file observation
// requirements and an O1/S2 checkpoint. Journal and admission provenance stay
// state-owned, and the returned value remains non-authorizing and read-only.
export function observeColimaLiveProviderStartFreshAdmissionForTest(
  argumentsValue,
) {
  const admittedArguments = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admittedArguments.requirements === null ||
    Array.isArray(admittedArguments.requirements) ||
    typeof admittedArguments.requirements !== "object" ||
    typeof admittedArguments.testCheckpoint !== "function"
  ) {
    fail("live provider fixture start admission arguments were refused", 64);
  }
  return observeColimaLiveProviderStartFreshAdmission(admittedArguments, {
    fixtureOnly: true,
    testCheckpoint: admittedArguments.testCheckpoint,
  });
}

function buildLiveProviderStartDecisionPublicationPlan(admission, snapshot) {
  try {
    return buildColimaLiveProviderStartDecisionPublicationPlan({
      admission,
      source: snapshot.source,
    });
  } catch (error) {
    if (error instanceof LiveProviderStartDecisionFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function sameLiveProviderStartDecisionValue(left, right) {
  return liveProviderStartDecisionBytes(left).equals(
    liveProviderStartDecisionBytes(right),
  );
}

function assertLiveProviderStartDecisionAdmissionPristine(admission) {
  if (
    admission.root_observation.root_set_disposition !== "observed-pristine" ||
    admission.process_start_decision_candidate === null
  ) {
    fail("live provider start decision namespaces were not pristine", 73);
  }
  return admission;
}

function completedLiveProviderStartDecisionForVariant(state, fixtureOnly) {
  const own = fixtureOnly
    ? state.liveProviderFixtureStartDecision
    : state.liveProviderStartDecision;
  const other = fixtureOnly
    ? state.liveProviderStartDecision
    : state.liveProviderFixtureStartDecision;
  if (other !== undefined) {
    fail("live provider start decision evidence class already differs", 73);
  }
  return own;
}

function assertLiveProviderStartDecisionVariantHistory(state, fixtureOnly) {
  for (const slot of state.mutationSlots) {
    if (slot.value.action !== COLIMA_LIVE_PROVIDER_START_DECISION_ACTION) {
      continue;
    }
    if (
      liveProviderStartDecisionFixtureOnly(
        slot.value.operation_kind,
        slot.value.operation_contract_sha256,
      ) !== fixtureOnly
    ) {
      fail("live provider start decision evidence classes were mixed", 73);
    }
  }
}

function validateLiveProviderStartDecisionPublicationStages(state, ownStage) {
  if (
    state.mutationStages.some(
      (stage) => stage.linkedDestination !== undefined,
    )
  ) {
    fail("live provider start decision publication stage was refused", 73);
  }
  if (ownStage === undefined) return;
  const staged = state.mutationStages.find(
    (stage) => stage.path === ownStage.stagePath,
  );
  if (
    staged === undefined ||
    staged.name !== ownStage.stagePath.split(sep).at(-1) ||
    !sameMetadata(staged.metadata, ownStage.identity)
  ) {
    fail("live provider start decision publication stage changed", 73);
  }
  const parsed = parseCanonical(
    ownStage.stagePath,
    "live provider start decision publication stage",
    1,
  );
  if (!parsed.bytes.equals(ownStage.bytes)) {
    fail("live provider start decision publication stage changed", 73);
  }
}

function availableLiveProviderStartDecisionSnapshot(
  state,
  publicationPlan,
  ownStage,
  fixtureOnly,
) {
  const snapshot = completedLiveProviderIntentCoreSnapshot(state, fixtureOnly);
  assertLiveProviderStartDecisionVariantHistory(state, fixtureOnly);
  const completedIntent = completedLiveProviderIntentForVariant(
    state,
    fixtureOnly,
  );
  const tail = abortedLiveProviderStartDecisionTail(state, completedIntent);
  const expectedLastClose =
    tail.length === 0
      ? completedIntent.close
      : state.mutationCloses.find(
          (close) =>
            close.value.slot_sequence === tail.at(-1).value.journal_sequence,
        );
  const expectedProviderContract = fixtureOnly
    ? "live-provider-fixture-intent-only"
    : "live-provider-intent-only";
  if (
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.mutationCloses.at(-1) !== expectedLastClose ||
    state.liveProviderStartDecision !== undefined ||
    state.liveProviderFixtureStartDecision !== undefined ||
    state.providerState.contract !== expectedProviderContract
  ) {
    fail("live provider start decision publication state was refused", 73);
  }
  validateLiveProviderStartDecisionPublicationStages(state, ownStage);
  if (ownStage !== undefined) {
    const staged = parseCanonical(
      ownStage.stagePath,
      "prospective live provider start decision slot",
      1,
    );
    validateMutationLeaseValue(staged.value, state.candidate.run_id);
    if (
      ownStage.destinationName !==
        mutationSlotFileName(state.mutationSlots.length) ||
      staged.value.action !== COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
      staged.value.journal_sequence !== state.mutationSlots.length ||
      staged.value.source_sequence !== 0 ||
      staged.value.source_head_sha256 !== state.receiptState.head_sha256 ||
      staged.value.source_environment_sha256 !== ZERO_SHA256 ||
      staged.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      staged.value.previous_close_sha256 !== digest(expectedLastClose.bytes) ||
      !canonicalBytes(staged.value.operation_plan).equals(
        canonicalBytes(publicationPlan),
      ) ||
      mutationOwnerState(staged.value) !== "current"
    ) {
      fail("prospective live provider start decision slot was refused", 73);
    }
  }
  return snapshot;
}

function heldLiveProviderStartDecisionSnapshot(
  state,
  held,
  publicationPlan,
  ownStage,
  fixtureOnly,
) {
  const snapshot = completedLiveProviderIntentCoreSnapshot(state, fixtureOnly);
  assertLiveProviderStartDecisionVariantHistory(state, fixtureOnly);
  const completedIntent = completedLiveProviderIntentForVariant(
    state,
    fixtureOnly,
  );
  const intentSequence = completedIntent.slot.value.journal_sequence;
  const tail = state.mutationSlots.slice(intentSequence + 1);
  const decisionSlot = tail.at(-1);
  const closes = new Map(
    state.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  const current = parseCanonical(held.path, "mutation slot");
  const currentIdentity = mutationHeldIdentity(held.path);
  const expectedProviderContract = fixtureOnly
    ? "live-provider-fixture-intent-only"
    : "live-provider-intent-only";
  if (
    decisionSlot === undefined ||
    decisionSlot !== state.mutationLease ||
    !decisionSlot.bytes.equals(held.leaseBytes) ||
    !current.bytes.equals(held.leaseBytes) ||
    !sameMetadata(currentIdentity, held.identity) ||
    decisionSlot.value.action !==
      COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
    !canonicalBytes(decisionSlot.value.operation_plan).equals(
      canonicalBytes(publicationPlan),
    ) ||
    tail
      .slice(0, -1)
      .some(
        (slot) =>
          slot.value.action !== COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
          closes.get(slot.value.journal_sequence)?.value.disposition !==
            "aborted-before-effect",
      ) ||
    state.mutationRecoveries.length !== 0 ||
    state.liveProviderStartDecision !== undefined ||
    state.liveProviderFixtureStartDecision !== undefined ||
    state.providerState.contract !== expectedProviderContract
  ) {
    fail("held live provider start decision state was refused", 73);
  }
  validateLiveProviderStartDecisionPublicationStages(state, ownStage);
  if (ownStage !== undefined) {
    const stagedClose = parseCanonical(
      ownStage.stagePath,
      "prospective live provider start decision close",
      1,
    );
    validateMutationCloseValue(
      stagedClose.value,
      decisionSlot,
      state.candidate.run_id,
    );
    if (
      ownStage.destinationName !==
        mutationCloseFileName(decisionSlot.value.journal_sequence) ||
      stagedClose.value.disposition !== "completed" ||
      stagedClose.value.authority !== "owner" ||
      stagedClose.value.result_sequence !== 0 ||
      stagedClose.value.result_head_sha256 !==
        state.receiptState.head_sha256 ||
      stagedClose.value.result_environment_sha256 !== ZERO_SHA256 ||
      stagedClose.value.operation_evidence_sha256 !== ZERO_SHA256
    ) {
      fail("prospective live provider start decision close was refused", 73);
    }
  }
  return snapshot;
}

function observeLiveProviderStartDecisionAdmission(
  argumentsValue,
  { boundary, fixtureOnly, stateSnapshot, testCheckpoint },
) {
  let observedSnapshot;
  const checkpoint =
    testCheckpoint === undefined
      ? undefined
      : (name) => testCheckpoint(`${boundary}:${name}`);
  const admission = observeColimaLiveProviderStartFreshAdmission(
    argumentsValue,
    {
      fixtureOnly,
      stateSnapshot(state) {
        observedSnapshot = stateSnapshot(state);
        return observedSnapshot;
      },
      testCheckpoint: checkpoint,
    },
  );
  if (observedSnapshot === undefined) {
    fail("live provider start decision state observation was unavailable", 70);
  }
  return Object.freeze({ admission, snapshot: observedSnapshot });
}

function liveProviderStartDecisionCompletion(completed) {
  try {
    return buildColimaLiveProviderStartDecisionCompletion({
      closeSha256: digest(completed.close.bytes),
      publicationPlan: completed.publicationPlan,
      slotSha256: digest(completed.slot.bytes),
      source: completed.source,
    });
  } catch (error) {
    if (error instanceof LiveProviderStartDecisionFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function completedLiveProviderStartDecisionCoreSnapshot(state, fixtureOnly) {
  const completed = completedLiveProviderStartDecisionForVariant(
    state,
    fixtureOnly,
  );
  if (completed === undefined) {
    fail("completed live provider start decision was unavailable", 69);
  }
  const intentSnapshot = completedLiveProviderIntentSourceSnapshot(
    state,
    fixtureOnly,
    "start-decision",
  );
  assertLiveProviderStartDecisionVariantHistory(state, fixtureOnly);
  const expectedProviderContract = fixtureOnly
    ? "live-provider-fixture-start-decision-only"
    : "live-provider-start-decision-only";
  if (
    completed === undefined ||
    completed.fixtureOnly !== fixtureOnly ||
    completed.close.value.slot_sequence !==
      completed.slot.value.journal_sequence ||
    completed.close.value.disposition !== "completed" ||
    completed.close.value.authority !== "owner" ||
    completed.close.value.result_sequence !== 0 ||
    completed.close.value.result_head_sha256 !==
      state.receiptState.head_sha256 ||
    completed.close.value.result_environment_sha256 !== ZERO_SHA256 ||
    completed.close.value.operation_evidence_sha256 !== ZERO_SHA256 ||
    state.mutationCloses.at(-1) !== completed.close ||
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.mutationStages.length !== 0 ||
    state.pendingPublication !== undefined ||
    state.environment !== undefined ||
    state.environmentPublication !== undefined ||
    state.mutationOperations.length !== 0 ||
    state.operationSettlement !== undefined ||
    state.cleanupSettlement !== undefined ||
    state.providerState.contract !== expectedProviderContract ||
    state.providerState.operationEvidenceSha256 !== ZERO_SHA256 ||
    state.cleanupState.contract !== "journal-only" ||
    state.cleanupState.operationEvidenceSha256 !== ZERO_SHA256 ||
    !liveProviderStartDecisionBytes(completed.source).equals(
      liveProviderStartDecisionBytes(intentSnapshot.source),
    )
  ) {
    fail("completed live provider start decision was unavailable", 69);
  }
  const source = Object.freeze({
    closeAuthority: completed.close.value.authority,
    fixtureOnly: completed.fixtureOnly,
    startDecisionCompletion: liveProviderStartDecisionCompletion(completed),
    startDecisionPublicationPlan: completed.publicationPlan,
    startDecisionSource: completed.source,
  });
  let completedStartDecisionProjection;
  try {
    completedStartDecisionProjection =
      buildColimaLiveCompletedProviderStartDecisionProjectionStructure(source);
  } catch (error) {
    if (error instanceof LiveProviderProcessStartFailure) {
      fail(
        "completed live provider start decision projection was refused",
        error.exitStatus,
      );
    }
    throw error;
  }
  return Object.freeze({
    ...intentSnapshot,
    completedStartDecisionProjection,
    source,
  });
}

function sameCompletedLiveProviderStartDecisionSnapshot(left, right) {
  return (
    sameLiveProviderPlanSnapshot(left, right) &&
    liveProviderStartDecisionBytes(left.completedIntentProjection).equals(
      liveProviderStartDecisionBytes(right.completedIntentProjection),
    ) &&
    left.source.closeAuthority === right.source.closeAuthority &&
    left.source.fixtureOnly === right.source.fixtureOnly &&
    liveProviderStartDecisionBytes(left.source.startDecisionCompletion).equals(
      liveProviderStartDecisionBytes(right.source.startDecisionCompletion),
    ) &&
    liveProviderStartDecisionBytes(
      left.source.startDecisionPublicationPlan,
    ).equals(
      liveProviderStartDecisionBytes(
        right.source.startDecisionPublicationPlan,
      ),
    ) &&
    liveProviderStartDecisionBytes(left.source.startDecisionSource).equals(
      liveProviderStartDecisionBytes(right.source.startDecisionSource),
    ) &&
    liveProviderProcessStartBytes(
      left.completedStartDecisionProjection,
    ).equals(
      liveProviderProcessStartBytes(
        right.completedStartDecisionProjection,
      ),
    )
  );
}

function buildLiveProviderProcessStartEffectFreshAdmission(
  snapshot,
  rootObservation,
) {
  try {
    return buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
      completedStartDecisionProjection:
        snapshot.completedStartDecisionProjection,
      rootObservation,
      source: snapshot.source,
    });
  } catch (error) {
    if (error instanceof LiveProviderProcessStartFailure) {
      fail("live provider process start admission was refused", error.exitStatus);
    }
    throw error;
  }
}

function observeColimaLiveProviderStartEffectFreshAdmission(
  admittedArguments,
  {
    fixtureOnly,
    stateSnapshot = (state) =>
      completedLiveProviderStartDecisionCoreSnapshot(state, fixtureOnly),
    testCheckpoint,
  },
) {
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const firstState = stateSnapshot(loadState(roots, true));
  const firstRoots = observeLivePreEffectRoots(
    admittedArguments,
    firstState,
    fixtureOnly,
  );
  validateLivePreEffectRootBinding(firstRoots, firstState, fixtureOnly);
  const firstAdmission = buildLiveProviderProcessStartEffectFreshAdmission(
    firstState,
    firstRoots,
  );
  testCheckpoint?.("after-first-process-start-effect-admission-observation");

  const secondState = stateSnapshot(loadState(roots, true));
  if (!sameCompletedLiveProviderStartDecisionSnapshot(firstState, secondState)) {
    fail("completed live provider start decision state changed", 73);
  }
  const secondRoots = observeLivePreEffectRoots(
    admittedArguments,
    secondState,
    fixtureOnly,
  );
  validateLivePreEffectRootBinding(secondRoots, secondState, fixtureOnly);
  if (!colimaLiveBytes(firstRoots).equals(colimaLiveBytes(secondRoots))) {
    fail("live provider process start root observation changed", 73);
  }
  const secondAdmission = buildLiveProviderProcessStartEffectFreshAdmission(
    secondState,
    secondRoots,
  );
  if (
    !liveProviderProcessStartBytes(firstAdmission).equals(
      liveProviderProcessStartBytes(secondAdmission),
    )
  ) {
    fail("live provider process start admission changed", 73);
  }
  return secondAdmission;
}

export function observeColimaLiveProviderStartEffectFreshAdmissionForExecutor(
  argumentsValue,
) {
  return observeColimaLiveProviderStartEffectFreshAdmission(
    admissionArguments(argumentsValue),
    { fixtureOnly: false, testCheckpoint: undefined },
  );
}

// This unsupported fixture seam substitutes only the bounded observation
// requirements and the O1/S2 checkpoint. State and decision provenance remain
// internally reconstructed, and the result grants no process authority.
export function observeColimaLiveProviderStartEffectFreshAdmissionForTest(
  argumentsValue,
) {
  const admittedArguments = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admittedArguments.requirements === null ||
    Array.isArray(admittedArguments.requirements) ||
    typeof admittedArguments.requirements !== "object" ||
    typeof admittedArguments.testCheckpoint !== "function"
  ) {
    fail("live provider fixture process start admission arguments were refused", 64);
  }
  return observeColimaLiveProviderStartEffectFreshAdmission(
    admittedArguments,
    {
      fixtureOnly: true,
      testCheckpoint: admittedArguments.testCheckpoint,
    },
  );
}

function callLiveProviderEffectCheckpoint(testCheckpoint, name) {
  if (
    testCheckpoint !== undefined &&
    testCheckpoint(name) !== undefined
  ) {
    fail("live provider effect checkpoint returned a value", 70);
  }
}

function completedLiveProviderStartDecisionEffectSnapshot(state) {
  const completed = state.liveProviderFixtureStartDecision;
  if (
    completed === undefined ||
    state.liveProviderStartDecision !== undefined ||
    state.mutationSlots.some(
      (slot) => slot.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
    )
  ) {
    fail("fixture provider effect start decision was unavailable", 69);
  }
  const decisionSequence = completed.slot.value.journal_sequence;
  const closes = new Map(
    state.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  const effectTail = state.mutationSlots.slice(decisionSequence + 1);
  if (
    effectTail.some(
      (slot) =>
        slot.value.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
        liveProviderEffectFixtureOnly(
          slot.value.operation_kind,
          slot.value.operation_contract_sha256,
        ) !== true ||
        closes.get(slot.value.journal_sequence)?.value.disposition !==
          "aborted-before-effect",
    ) ||
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.mutationStages.length !== 0 ||
    state.providerEffectStage !== undefined ||
    state.providerEffectWitness !== undefined ||
    state.providerEffectEvents.length !== 0 ||
    state.mutationOperations.length !== 0
  ) {
    fail("fixture provider effect start decision was unavailable", 69);
  }
  const historicalState = {
    ...state,
    liveProviderFixtureEffect: undefined,
    mutationCloses: state.mutationCloses.filter(
      (close) => close.value.slot_sequence <= decisionSequence,
    ),
    mutationSlots: state.mutationSlots.slice(0, decisionSequence + 1),
    providerState: Object.freeze({
      contract: "live-provider-fixture-start-decision-only",
      operationEvidenceSha256: ZERO_SHA256,
    }),
  };
  return completedLiveProviderStartDecisionCoreSnapshot(
    historicalState,
    true,
  );
}

function availableLiveProviderEffectStartDecisionSnapshot(
  state,
  publicationPlan,
  ownStage,
) {
  if (
    state.mutationStages.some(
      (stage) => stage.linkedDestination !== undefined,
    )
  ) {
    fail("fixture provider effect slot stage was refused", 73);
  }
  if (ownStage === undefined) {
    if (state.mutationStages.length !== 0) {
      fail("fixture provider effect slot stage was refused", 73);
    }
  } else {
    const stage = state.mutationStages.find(
      (candidate) => candidate.path === ownStage.stagePath,
    );
    if (
      stage === undefined ||
      stage.name !== ownStage.stagePath.split(sep).at(-1) ||
      !sameMetadata(stage.metadata, ownStage.identity)
    ) {
      fail("fixture provider effect slot stage changed", 73);
    }
    const staged = parseCanonical(
      ownStage.stagePath,
      "prospective fixture provider effect slot",
      1,
    );
    validateMutationLeaseValue(staged.value, state.candidate.run_id);
    const previousClose = state.mutationCloses.at(-1);
    if (
      ownStage.destinationName !==
        mutationSlotFileName(state.mutationSlots.length) ||
      staged.value.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
      staged.value.journal_sequence !== state.mutationSlots.length ||
      staged.value.source_sequence !== 0 ||
      staged.value.source_head_sha256 !== state.receiptState.head_sha256 ||
      staged.value.source_environment_sha256 !== ZERO_SHA256 ||
      staged.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      staged.value.previous_close_sha256 !== digest(previousClose.bytes) ||
      staged.value.operation_kind !== publicationPlan.operation_kind ||
      staged.value.operation_contract_sha256 !==
        publicationPlan.operation_contract_sha256 ||
      !liveProviderEffectBytes(staged.value.operation_plan).equals(
        liveProviderEffectBytes(publicationPlan),
      ) ||
      mutationOwnerState(staged.value) !== "current"
    ) {
      fail("prospective fixture provider effect slot was refused", 73);
    }
  }
  return completedLiveProviderStartDecisionEffectSnapshot({
    ...state,
    mutationStages: [],
  });
}

function effectCommitment(seed, label) {
  return liveProviderEffectDigest(
    liveProviderEffectBytes({
      domain: "synveda-cpr45-state-owned-fixture-effect-v1",
      label,
      seed,
    }),
  );
}

function buildLiveProviderEffectPublicationPlanForState(
  admission,
  observed,
  snapshot,
) {
  const seed = liveProviderEffectDigest(liveProviderEffectBytes({
    admission_sha256: liveProviderEffectDigest(
      liveProviderEffectBytes(admission),
    ),
    fixture_id: snapshot.operationPlan.fixture_id,
    provider_root_identity: observed.provider_root_identity,
    state_run_identity: observed.state_run_identity,
  }));
  const invocationBinding = Object.freeze({
    argv_sha256: effectCommitment(seed, "fixed-fixture-argv"),
    cwd_identity_sha256: effectCommitment(seed, "fixed-fixture-cwd"),
    environment_sha256: effectCommitment(seed, "fixed-fixture-environment"),
    executable_sha256: effectCommitment(seed, "fixed-fixture-executable"),
    toolchain_sha256: effectCommitment(seed, "fixed-fixture-toolchain"),
  });
  const plannedRoleContracts = COLIMA_LIVE_PROVIDER_EFFECT_ROLES.map(
    (role) => ({
      argv_sha256:
        role === "outer"
          ? invocationBinding.argv_sha256
          : effectCommitment(seed, `${role}-argv`),
      cwd_identity_sha256:
        role === "outer"
          ? invocationBinding.cwd_identity_sha256
          : effectCommitment(seed, `${role}-cwd`),
      depth: role === "outer" ? 0 : role === "hostagent" ? 1 : 2,
      environment_sha256:
        role === "outer"
          ? invocationBinding.environment_sha256
          : effectCommitment(seed, `${role}-environment`),
      executable_sha256:
        role === "outer"
          ? invocationBinding.executable_sha256
          : effectCommitment(seed, `${role}-executable`),
      parent_role:
        role === "outer"
          ? "state-owner"
          : role === "hostagent"
            ? "outer"
            : "hostagent",
      public_key_spki_sha256: effectCommitment(seed, `${role}-public-key`),
      role,
      role_challenge_sha256: effectCommitment(seed, `${role}-challenge`),
      toolchain_sha256:
        role === "outer"
          ? invocationBinding.toolchain_sha256
          : effectCommitment(seed, `${role}-toolchain`),
      uid: observed.provider_root_identity.uid,
    }),
  );
  const endpointRoles = Object.freeze({
    "engine-api": "hostagent",
    "hostagent-control": "hostagent",
    "ssh-control": "ssh-controlmaster",
    "usernet-control": "usernet",
  });
  const endpointNamespaces = Object.freeze({
    "engine-api": "colima-home-namespace",
    "hostagent-control": "lima-home-namespace",
    "ssh-control": "temporary-namespace",
    "usernet-control": "lima-home-namespace",
  });
  const plannedEndpointContracts = COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.map(
    (endpointKind) => ({
      docker_context_namespace_role:
        endpointKind === "engine-api" ? "docker-config-namespace" : "none",
      docker_context_path_identity_hmac_sha256:
        endpointKind === "engine-api"
          ? effectCommitment(seed, "engine-api-docker-context-path")
          : ZERO_SHA256,
      docker_context_sha256:
        endpointKind === "engine-api"
          ? effectCommitment(seed, "engine-api-docker-context")
          : ZERO_SHA256,
      endpoint_kind: endpointKind,
      final_challenge_commitment_sha256: effectCommitment(
        seed,
        `${endpointKind}-final-challenge`,
      ),
      initial_challenge_commitment_sha256: effectCommitment(
        seed,
        `${endpointKind}-initial-challenge`,
      ),
      namespace_role: endpointNamespaces[endpointKind],
      path_identity_hmac_sha256: effectCommitment(
        seed,
        `${endpointKind}-path`,
      ),
      role: endpointRoles[endpointKind],
    }),
  );
  try {
    return buildColimaLiveProviderEffectPublicationPlan({
      admission,
      invocationBinding,
      namespaceBindings: observed.namespace_bindings.map((binding) => ({
        device: binding.device,
        inode: binding.inode,
        mode: binding.mode,
        role: binding.role,
        uid: binding.uid,
      })),
      plannedEndpointContracts,
      plannedQuiescenceFenceSha256: effectCommitment(
        seed,
        "quiescence-fence",
      ),
      plannedRoleContracts,
      plannedStartAttemptSha256: effectCommitment(seed, "start-attempt"),
      providerRootIdentity: observed.provider_root_identity,
      source: snapshot.source,
      stateRunIdentity: observed.state_run_identity,
    });
  } catch (error) {
    if (error instanceof LiveProviderEffectFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function fixtureEffectArguments(argumentsValue) {
  const admitted = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admitted.requirements === null ||
    Array.isArray(admitted.requirements) ||
    typeof admitted.requirements !== "object" ||
    typeof admitted.testCheckpoint !== "function"
  ) {
    fail("fixture provider effect arguments were refused", 64);
  }
  return admitted;
}

function observeLiveProviderEffectPhysicalRoots(
  argumentsValue,
  state,
  source,
  publicationPlan,
  markerDisposition,
) {
  const observed = observeLiveProviderReservationPhysicalRoots(
    argumentsValue,
    state,
    source,
    undefined,
    true,
    markerDisposition,
    undefined,
  );
  if (publicationPlan === undefined) return observed;
  const physicalNamespaces = publicationPlan.namespace_bindings.map(
    (binding) => ({
      device: binding.device,
      inode: binding.inode,
      mode: binding.mode,
      namespace_identity_hmac_sha256:
        binding.namespace_identity_hmac_sha256,
      observed_entry_set_hmac_sha256:
        binding.observed_entry_set_hmac_sha256,
      role: binding.role,
      uid: binding.uid,
    }),
  );
  if (
    observed.marker_disposition !== markerDisposition ||
    !liveProviderEffectBytes(observed.root_observation).equals(
      liveProviderEffectBytes(publicationPlan.admission.root_observation),
    ) ||
    !liveProviderEffectBytes(observed.provider_root_identity).equals(
      liveProviderEffectBytes(publicationPlan.provider_root_identity),
    ) ||
    !liveProviderEffectBytes(observed.state_run_identity).equals(
      liveProviderEffectBytes(publicationPlan.state_run_identity),
    ) ||
    !liveProviderEffectBytes(observed.namespace_bindings).equals(
      liveProviderEffectBytes(physicalNamespaces),
    )
  ) {
    fail("fixture provider effect root observation changed", 73);
  }
  return observed;
}

function assertLiveProviderEffectOpenState(state, publicationPlan) {
  const effect = state.liveProviderFixtureEffect;
  if (
    effect === undefined ||
    effect.close !== undefined ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(effect.slot.bytes) ||
    !liveProviderEffectBytes(effect.publicationPlan).equals(
      liveProviderEffectBytes(publicationPlan),
    ) ||
    state.receiptState.head.sequence !== effect.slot.value.source_sequence ||
    state.receiptState.head_sha256 !== effect.slot.value.source_head_sha256 ||
    state.environment !== undefined ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined ||
    state.mutationOperations.length !== 0 ||
    state.providerState.contract !== "live-provider-fixture-effect-only" ||
    state.cleanupState.contract !== "journal-only"
  ) {
    fail("fixture provider effect state changed", 73);
  }
  return effect;
}

function assertLiveProviderEffectBoundDirectories(
  state,
  publicationPlan,
  argumentsValue,
) {
  const runMetadata = reservationDirectoryIdentity(
    state.run,
    publicationPlan.state_run_identity,
    "fixture provider effect state run",
  );
  const providerRootMetadata = reservationDirectoryIdentity(
    argumentsValue.observationInput.provider_root,
    publicationPlan.provider_root_identity,
    "fixture provider effect provider root",
  );
  if (runMetadata.dev !== providerRootMetadata.dev) {
    fail("fixture provider effect filesystem changed", 73);
  }
  return Object.freeze({ providerRootMetadata, runMetadata });
}

function assertLiveProviderEffectMarkerAbsent(
  state,
  publicationPlan,
  argumentsValue,
) {
  assertLiveProviderEffectBoundDirectories(
    state,
    publicationPlan,
    argumentsValue,
  );
  if (
    !liveProviderReservationMarkerIsAbsent(
      liveProviderReservationMarkerPath(argumentsValue),
    )
  ) {
    fail("fixture provider effect marker was not absent", 73);
  }
}

function assertLiveProviderEffectLinkedTopology(
  state,
  publicationPlan,
  argumentsValue,
  expectedLinks,
) {
  const { providerRootMetadata, runMetadata } =
    assertLiveProviderEffectBoundDirectories(
      state,
      publicationPlan,
      argumentsValue,
    );
  const artifacts = [state.providerEffectStage, state.providerEffectWitness]
    .filter((artifact) => artifact !== undefined);
  if (
    artifacts.length === 0 ||
    artifacts.some(
      (artifact) =>
        artifact.initializing === true ||
        artifact.metadata.nlink !== BigInt(expectedLinks),
    )
  ) {
    fail("fixture provider effect local topology changed", 73);
  }
  const expectedBytes = artifacts[0].bytes;
  const expectedIdentity = artifacts[0].metadata;
  for (const artifact of artifacts) {
    const metadata = inspectLiveProviderReservationArtifact(
      artifact.path,
      "fixture provider effect state artifact",
      runMetadata.dev,
      expectedLinks,
      expectedBytes,
      expectedIdentity,
    );
    if (!sameMutationArtifact(metadata, artifact.metadata)) {
      fail("fixture provider effect state artifact changed", 73);
    }
  }
  const marker = inspectLiveProviderReservationArtifact(
    liveProviderReservationMarkerPath(argumentsValue),
    "fixture provider effect marker",
    providerRootMetadata.dev,
    expectedLinks,
    expectedBytes,
    expectedIdentity,
  );
  if (!sameMutationArtifact(marker, expectedIdentity)) {
    fail("fixture provider effect marker identity changed", 73);
  }
  return Object.freeze({ bytes: expectedBytes, identity: expectedIdentity });
}

function assertLiveProviderEffectRetiredWitness(
  state,
  publicationPlan,
  argumentsValue,
) {
  const { runMetadata } = assertLiveProviderEffectBoundDirectories(
    state,
    publicationPlan,
    argumentsValue,
  );
  const witness = state.providerEffectWitness;
  if (
    state.providerEffectStage !== undefined ||
    witness === undefined ||
    witness.metadata.nlink !== 1n
  ) {
    fail("fixture provider effect retirement was not durable", 73);
  }
  inspectLiveProviderReservationArtifact(
    witness.path,
    "fixture provider effect witness",
    runMetadata.dev,
    1,
    witness.bytes,
    witness.metadata,
  );
  assertLiveProviderEffectMarkerAbsent(
    state,
    publicationPlan,
    argumentsValue,
  );
  return witness;
}

function writeLiveProviderEffectStage(
  state,
  publicationPlan,
  held,
  source,
  testCheckpoint,
) {
  const nonce = randomBytes(16).toString("hex");
  const name = providerEffectStageFileName(
    held.lease.journal_sequence,
    nonce,
  );
  const path = join(state.run, name);
  const runMetadata = ownedPrivateDirectory(state.run, "active run state");
  let descriptor;
  let witness;
  let bytes;
  let identity;
  try {
    descriptor = openSync(
      path,
      constants.O_WRONLY |
        constants.O_CREAT |
        constants.O_EXCL |
        constants.O_NOFOLLOW,
      0o600,
    );
    identity = exactFstat(descriptor);
    if (
      !identity.isFile() ||
      identity.uid !== OWNER_UID ||
      identity.dev !== runMetadata.dev ||
      identity.nlink !== 1n ||
      (identity.mode & 0o7777n) !== 0o600n ||
      identity.size !== 0n
    ) {
      fail("fixture provider effect stage creation was refused");
    }
    fsyncSync(descriptor);
    syncDirectory(state.run);
    callLiveProviderEffectCheckpoint(
      testCheckpoint,
      "after-witness-stage-create",
    );
    witness = buildColimaLiveProviderEffectWitness({
      markerWitnessIdentitySha256:
        providerEffectMarkerWitnessIdentitySha256(
          identity,
          digest(held.leaseBytes),
        ),
      publicationPlan,
      slotSequence: held.lease.journal_sequence,
      slotSha256: digest(held.leaseBytes),
      source,
    });
    bytes = liveProviderEffectBytes(witness);
    let offset = 0;
    while (offset < bytes.length) {
      const written = writeSync(
        descriptor,
        bytes,
        offset,
        bytes.length - offset,
      );
      if (written < 1) {
        fail("fixture provider effect stage write failed", 70);
      }
      offset += written;
    }
    fsyncSync(descriptor);
    const writtenIdentity = exactFstat(descriptor);
    if (
      writtenIdentity.dev !== identity.dev ||
      writtenIdentity.ino !== identity.ino ||
      writtenIdentity.uid !== identity.uid ||
      writtenIdentity.mode !== identity.mode ||
      writtenIdentity.nlink !== 1n ||
      writtenIdentity.size !== BigInt(bytes.length)
    ) {
      fail("fixture provider effect stage identity changed", 73);
    }
    identity = writtenIdentity;
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail("fixture provider effect stage publication failed", 70);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  syncDirectory(state.run);
  const verifiedIdentity = inspectLiveProviderReservationArtifact(
    path,
    "fixture provider effect stage",
    runMetadata.dev,
    1,
    bytes,
    identity,
  );
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-witness-stage");
  return Object.freeze({
    bytes,
    identity: verifiedIdentity,
    name,
    path,
    value: witness,
  });
}

function linkLiveProviderEffectMarker(
  state,
  stage,
  argumentsValue,
  testCheckpoint,
) {
  try {
    linkSync(
      stage.path,
      liveProviderReservationMarkerPath(argumentsValue),
    );
  } catch (error) {
    if (error?.code === "EEXIST") return false;
    fail("fixture provider effect marker publication failed", 70);
  }
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-marker-link");
  syncDirectory(argumentsValue.observationInput.provider_root);
  callLiveProviderEffectCheckpoint(
    testCheckpoint,
    "after-marker-directory-sync",
  );
  return true;
}

function linkLiveProviderEffectWitness(
  state,
  stage,
  witnessPath,
  testCheckpoint,
) {
  try {
    linkSync(stage.path, witnessPath);
  } catch {
    fail("fixture provider effect witness publication failed", 70);
  }
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-state-witness-link");
  syncDirectory(state.run);
  callLiveProviderEffectCheckpoint(
    testCheckpoint,
    "after-state-witness-directory-sync",
  );
}

function unlinkLiveProviderEffectStage(
  state,
  stage,
  expectedLinks,
  testCheckpoint,
) {
  const runMetadata = ownedPrivateDirectory(state.run, "active run state");
  const expectedBytes = stage.bytes;
  const metadata = inspectPendingFile(
    stage.path,
    "fixture provider effect stage",
    runMetadata.dev,
    new Set([BigInt(expectedLinks)]),
  );
  if (
    !sameMutationArtifact(metadata, stage.identity ?? stage.metadata) ||
    !readPrivate(
      stage.path,
      "fixture provider effect stage",
      expectedLinks,
      0n,
    ).equals(expectedBytes)
  ) {
    fail("fixture provider effect stage identity changed", 73);
  }
  try {
    unlinkSync(stage.path);
  } catch {
    fail("fixture provider effect stage retirement failed", 70);
  }
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-witness-stage-unlink");
  syncDirectory(state.run);
  callLiveProviderEffectCheckpoint(
    testCheckpoint,
    "after-witness-stage-directory-sync",
  );
}

function liveProviderEffectValueDigest(value) {
  return liveProviderEffectDigest(liveProviderEffectBytes(value));
}

function observeLiveProviderEffectStartReproof(
  roots,
  argumentsValue,
  publicationPlan,
  source,
  witness,
  observationSequence,
  assertAuthority,
) {
  let state = assertAuthority();
  assertLiveProviderEffectLinkedTopology(
    state,
    publicationPlan,
    argumentsValue,
    2,
  );
  const observed = observeLiveProviderEffectPhysicalRoots(
    argumentsValue,
    state,
    source,
    publicationPlan,
    "present",
  );
  state = assertAuthority();
  assertLiveProviderEffectLinkedTopology(
    state,
    publicationPlan,
    argumentsValue,
    2,
  );
  const observationChallengeSha256 = liveProviderEffectDigest(
    Buffer.concat([
      liveProviderEffectBytes({
        marker_disposition: observed.marker_disposition,
        namespace_bindings: observed.namespace_bindings,
        provider_root_identity: observed.provider_root_identity,
        root_observation: observed.root_observation,
        state_run_identity: observed.state_run_identity,
      }),
      randomBytes(32),
    ]),
  );
  return Object.freeze({
    admission_sha256: liveProviderEffectValueDigest(publicationPlan.admission),
    invocation_sha256: liveProviderEffectValueDigest(
      publicationPlan.invocation_binding,
    ),
    marker_link_count: 2,
    marker_present: true,
    marker_witness_identity_sha256:
      witness.marker_witness_identity_sha256,
    marker_witness_same_inode: true,
    namespace_sha256: liveProviderEffectValueDigest(
      publicationPlan.namespace_bindings,
    ),
    observation_challenge_sha256: observationChallengeSha256,
    observation_sequence: observationSequence,
    provider_root_identity_sha256: liveProviderEffectValueDigest(
      publicationPlan.provider_root_identity,
    ),
    source_sha256: liveProviderEffectValueDigest(source),
    state_run_identity_sha256: liveProviderEffectValueDigest(
      publicationPlan.state_run_identity,
    ),
    topology_sha256: liveProviderEffectValueDigest({
      endpoints: publicationPlan.planned_endpoint_contracts,
      roles: publicationPlan.planned_role_contracts,
    }),
    witness_link_count: 2,
  });
}

function publishLiveProviderEffectEvent(
  roots,
  state,
  argumentsValue,
  eventKind,
  evidence,
  assertAuthority,
  testCheckpoint,
) {
  if (typeof assertAuthority !== "function") {
    fail("fixture provider effect event authority was refused", 70);
  }
  const effect = assertLiveProviderEffectOpenState(
    state,
    state.liveProviderFixtureEffect.publicationPlan,
  );
  const witness = effect.witness;
  if (witness === undefined) {
    fail("fixture provider effect event lacked its witness", 73);
  }
  const publisherAuthorityChain = providerEffectPublisherAuthorityChain(
    effect.events,
    effect.recoveries,
    witness,
    { includePending: true },
  );
  const publisherAuthoritySha256 =
    effect.recoveries.length === 0
      ? digest(effect.slot.bytes)
      : digest(effect.recoveries.at(-1).bytes);
  let event;
  try {
    event = buildColimaLiveProviderEffectEvent({
      eventKind,
      evidence,
      history: effect.events.map((entry) => entry.value),
      publicationPlan: effect.publicationPlan,
      publisherAuthorityChain,
      publisherAuthoritySha256,
      source: effect.source,
      witness: witness.value,
    });
  } catch (error) {
    if (error instanceof LiveProviderEffectFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
  const bytes = liveProviderEffectBytes(event);
  const name = providerEffectEventFileName(
    effect.slot.value.journal_sequence,
    effect.events.length,
  );
  const reassertEventPublication = () => {
    const current = assertAuthority();
    const currentEffect = assertLiveProviderEffectOpenState(
      current,
      effect.publicationPlan,
    );
    if (
      currentEffect.events.length !== effect.events.length ||
      currentEffect.events.some(
        (entry, index) => !entry.bytes.equals(effect.events[index].bytes),
      ) ||
      !currentEffect.witness?.bytes.equals(witness.bytes) ||
      !currentEffect.slot.bytes.equals(effect.slot.bytes)
    ) {
      fail("fixture provider effect event source changed", 73);
    }
    if (eventKind === "completion") {
      if (currentEffect.currentStage !== "witness-unlinked") {
        fail("fixture provider effect completion source changed", 73);
      }
      assertLiveProviderEffectRetiredWitness(
        current,
        effect.publicationPlan,
        argumentsValue,
      );
    } else {
      assertLiveProviderEffectLinkedTopology(
        current,
        effect.publicationPlan,
        argumentsValue,
        2,
      );
    }
  };
  let afterLinkFailure;
  if (!publishMutationBlocker(state.run, name, bytes, {
    afterLinkObserver:
      testCheckpoint === undefined
        ? undefined
        : () => {
            try {
              callLiveProviderEffectCheckpoint(
                testCheckpoint,
                `after-${eventKind}-link`,
              );
            } catch (error) {
              afterLinkFailure = error;
            }
          },
    reassertAuthority: reassertEventPublication,
  })) {
    const existing = parseCanonical(
      join(state.run, name),
      "fixture provider effect event",
    );
    if (!existing.bytes.equals(bytes)) {
      fail("fixture provider effect event publication conflicted", 73);
    }
  }
  const verified = assertAuthority();
  const recorded = verified.liveProviderFixtureEffect?.events.at(-1);
  if (recorded === undefined || !recorded.bytes.equals(bytes)) {
    fail("fixture provider effect event was not durable", 70);
  }
  if (afterLinkFailure !== undefined) throw afterLinkFailure;
  callLiveProviderEffectCheckpoint(testCheckpoint, `after-${eventKind}`);
  return Object.freeze({ event: recorded, state: verified });
}

function retireLiveProviderEffectMarker(
  roots,
  state,
  argumentsValue,
  assertAuthority,
  testCheckpoint,
) {
  const effect = assertLiveProviderEffectOpenState(
    state,
    state.liveProviderFixtureEffect.publicationPlan,
  );
  const held = assertLiveProviderEffectLinkedTopology(
    state,
    effect.publicationPlan,
    argumentsValue,
    2,
  );
  const markerPath = liveProviderReservationMarkerPath(argumentsValue);
  inspectLiveProviderReservationArtifact(
    markerPath,
    "fixture provider effect marker",
    BigInt(effect.publicationPlan.provider_root_identity.device),
    2,
    held.bytes,
    held.identity,
  );
  const markerRetirementObservationSha256 = liveProviderEffectDigest(
    Buffer.concat([
      liveProviderEffectBytes({
        device: String(held.identity.dev),
        inode: String(held.identity.ino),
        links: "2",
        marker: effect.publicationPlan.fixed_marker_name,
        slot_sha256: digest(effect.slot.bytes),
      }),
      randomBytes(32),
    ]),
  );
  try {
    unlinkSync(markerPath);
  } catch {
    fail("fixture provider effect marker retirement failed", 70);
  }
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-marker-unlink");
  syncDirectory(argumentsValue.observationInput.provider_root);
  const providerRootFsyncObservationSha256 = liveProviderEffectDigest(
    Buffer.concat([
      liveProviderEffectBytes({
        marker_retirement_observation_sha256:
          markerRetirementObservationSha256,
        provider_root_identity: effect.publicationPlan.provider_root_identity,
        state: "marker-absent-parent-fsynced",
      }),
      randomBytes(32),
    ]),
  );
  callLiveProviderEffectCheckpoint(
    testCheckpoint,
    "after-marker-retirement-directory-sync",
  );
  const verified = assertAuthority();
  assertLiveProviderEffectRetiredWitness(
    verified,
    effect.publicationPlan,
    argumentsValue,
  );
  return Object.freeze({
    markerRetirementObservationSha256,
    providerRootFsyncObservationSha256,
    state: verified,
  });
}

function liveProviderEffectPreAttemptCompletionEvidence(
  effect,
  publisherAuthoritySha256,
  retirement,
) {
  const startAuthority = effect.events.find(
    (event) => event.value.event_kind === "start-authority",
  );
  const evidence = {
    cleanup_settlement_sha256: ZERO_SHA256,
    create_settlement_sha256: ZERO_SHA256,
    marker_present: false,
    marker_retirement_binding_sha256: ZERO_SHA256,
    marker_retirement_observation_sha256:
      retirement.markerRetirementObservationSha256,
    provider_root_fsync_observation_sha256:
      retirement.providerRootFsyncObservationSha256,
    receipt_event_sha256: ZERO_SHA256,
    receipt_sha256: ZERO_SHA256,
    start_authority_sha256:
      startAuthority === undefined ? ZERO_SHA256 : digest(startAuthority.bytes),
    start_attempt_sha256: ZERO_SHA256,
    variant: "pre-attempt-retired-zero-receipt",
    witness_identity_sha256:
      effect.witness.value.marker_witness_identity_sha256,
    witness_link_count: 1,
  };
  evidence.marker_retirement_binding_sha256 = liveProviderEffectValueDigest({
    cleanup_settlement_sha256: evidence.cleanup_settlement_sha256,
    marker_retirement_observation_sha256:
      evidence.marker_retirement_observation_sha256,
    provider_root_fsync_observation_sha256:
      evidence.provider_root_fsync_observation_sha256,
    publisher_authority_sha256: publisherAuthoritySha256,
    receipt_sha256: evidence.receipt_sha256,
    witness_identity_sha256: evidence.witness_identity_sha256,
    witness_link_count: evidence.witness_link_count,
  });
  return Object.freeze(evidence);
}

function publishColimaLiveProviderFixtureEffect(argumentsValue, terminal) {
  if (!new Set(["attempt-fenced", "pre-attempt-retired"]).has(terminal)) {
    fail("fixture provider effect terminal was refused", 64);
  }
  const admittedArguments = fixtureEffectArguments(argumentsValue);
  const { testCheckpoint } = admittedArguments;
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const validateCompleted = (state) => {
    const effect = state.liveProviderFixtureEffect;
    if (effect?.close?.value.disposition !== "completed") {
      fail("fixture provider effect completion changed", 73);
    }
    if (terminal !== "pre-attempt-retired") {
      fail("fixture provider effect terminal differed", 73);
    }
    validateLiveProviderReservationArgumentBinding(
      admittedArguments,
      effect.source,
      true,
    );
    assertLiveProviderEffectRetiredWitness(
      state,
      effect.publicationPlan,
      admittedArguments,
    );
    if (
      effect.completion?.value.evidence.variant !==
      "pre-attempt-retired-zero-receipt"
    ) {
      fail("fixture provider effect completion differed", 73);
    }
    return effect;
  };
  let initial = loadState(roots, true);
  let completed = initial.liveProviderFixtureEffect;
  if (completed?.close?.value.disposition === "completed") {
    validateCompleted(initial);
    reconcileMutationStages(roots);
    initial = loadState(roots, true);
    completed = validateCompleted(initial);
    return completed.completion.value;
  }
  reconcileMutationStages(roots);
  initial = loadState(roots, true);
  const snapshot = completedLiveProviderStartDecisionEffectSnapshot(initial);
  const admission = observeColimaLiveProviderStartEffectFreshAdmission(
    admittedArguments,
    {
      fixtureOnly: true,
      stateSnapshot: completedLiveProviderStartDecisionEffectSnapshot,
      testCheckpoint,
    },
  );
  const observed = observeLiveProviderEffectPhysicalRoots(
    admittedArguments,
    initial,
    snapshot.source,
    undefined,
    "absent",
  );
  if (
    !liveProviderEffectBytes(observed.root_observation).equals(
      liveProviderEffectBytes(admission.root_observation),
    )
  ) {
    fail("fixture provider effect admission changed", 73);
  }
  const publicationPlan = buildLiveProviderEffectPublicationPlanForState(
    admission,
    observed,
    snapshot,
  );
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-effect-plan");
  const reassertSlotPublication = (ownStage) => {
    const current = loadState(roots, true);
    const currentSnapshot = availableLiveProviderEffectStartDecisionSnapshot(
      current,
      publicationPlan,
      ownStage,
    );
    if (
      !sameCompletedLiveProviderStartDecisionSnapshot(
        snapshot,
        currentSnapshot,
      )
    ) {
      fail("fixture provider effect start decision changed", 73);
    }
    assertLiveProviderEffectMarkerAbsent(
      current,
      publicationPlan,
      admittedArguments,
    );
    observeLiveProviderEffectPhysicalRoots(
      admittedArguments,
      current,
      snapshot.source,
      publicationPlan,
      "absent",
    );
  };
  const expectedSource = Object.freeze({
    sequence: initial.receiptState.head.sequence,
    sha256: initial.receiptState.head_sha256,
  });
  let afterSlotLinkFailure;
  const held = acquireMutationLease(
    roots,
    COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
    ZERO_SHA256,
    expectedSource,
    {
      afterLinkObserver: () => {
        try {
          callLiveProviderEffectCheckpoint(
            testCheckpoint,
            "after-effect-slot-link",
          );
        } catch (error) {
          afterSlotLinkFailure = error;
        }
      },
      reassertAuthority: reassertSlotPublication,
    },
    Object.freeze({
      contractSha256: publicationPlan.operation_contract_sha256,
      kind: publicationPlan.operation_kind,
      plan: publicationPlan,
    }),
  );
  if (afterSlotLinkFailure !== undefined) throw afterSlotLinkFailure;
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-effect-slot");
  const assertOwner = () => {
    const current = assertMutationLeaseHeld(roots, held, false);
    assertLiveProviderEffectOpenState(current, publicationPlan);
    return current;
  };
  let state = assertOwner();
  let effect = assertLiveProviderEffectOpenState(state, publicationPlan);
  assertLiveProviderEffectMarkerAbsent(
    state,
    publicationPlan,
    admittedArguments,
  );
  observeLiveProviderEffectPhysicalRoots(
    admittedArguments,
    state,
    effect.source,
    publicationPlan,
    "absent",
  );
  const stage = writeLiveProviderEffectStage(
    state,
    publicationPlan,
    held,
    effect.source,
    testCheckpoint,
  );
  state = assertOwner();
  effect = assertLiveProviderEffectOpenState(state, publicationPlan);
  if (
    effect.stage?.initializing === true ||
    !effect.stage?.bytes.equals(stage.bytes)
  ) {
    fail("fixture provider effect stage was not durable", 70);
  }
  assertLiveProviderEffectMarkerAbsent(
    state,
    publicationPlan,
    admittedArguments,
  );
  observeLiveProviderEffectPhysicalRoots(
    admittedArguments,
    state,
    effect.source,
    publicationPlan,
    "absent",
  );
  callLiveProviderEffectCheckpoint(testCheckpoint, "before-marker-link");
  if (
    !linkLiveProviderEffectMarker(
      state,
      stage,
      admittedArguments,
      testCheckpoint,
    )
  ) {
    unlinkLiveProviderEffectStage(state, stage, 1, testCheckpoint);
    closeMutationLease(
      roots,
      held,
      "aborted-before-effect",
      {},
      ZERO_SHA256,
      (current) => {
        assertLiveProviderEffectOpenState(current, publicationPlan);
      },
    );
    fail("fixture provider effect marker was already held", 73);
  }
  state = assertOwner();
  assertLiveProviderEffectLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    2,
  );
  const witnessPath = join(
    state.run,
    providerEffectWitnessFileName(held.lease.journal_sequence),
  );
  linkLiveProviderEffectWitness(
    state,
    stage,
    witnessPath,
    testCheckpoint,
  );
  state = assertOwner();
  assertLiveProviderEffectLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    3,
  );
  unlinkLiveProviderEffectStage(state, stage, 3, testCheckpoint);
  state = assertOwner();
  effect = assertLiveProviderEffectOpenState(state, publicationPlan);
  const linkedWitness = assertLiveProviderEffectLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    2,
  );
  if (!effect.witness?.bytes.equals(linkedWitness.bytes)) {
    fail("fixture provider effect witness changed", 73);
  }
  callLiveProviderEffectCheckpoint(testCheckpoint, "before-start-authority-reproof");
  const firstReproof = observeLiveProviderEffectStartReproof(
    roots,
    admittedArguments,
    publicationPlan,
    effect.source,
    effect.witness.value,
    0,
    assertOwner,
  );
  const authorityEvidence = Object.freeze({
    authority_scope: "current-in-memory-owner-attempt-publication-only",
    current_owner_boot_sha256: held.lease.owner_boot_sha256,
    current_owner_instance_sha256: held.lease.owner_instance_sha256,
    first_reproof: firstReproof,
    first_reproof_sha256: liveProviderEffectValueDigest(firstReproof),
    recovery_invocation_authorized: false,
    serialized_invocation_authorized: false,
    witness_sha256: digest(effect.witness.bytes),
  });
  let published = publishLiveProviderEffectEvent(
    roots,
    state,
    admittedArguments,
    "start-authority",
    authorityEvidence,
    assertOwner,
    testCheckpoint,
  );
  state = published.state;
  const authorityEvent = published.event;
  effect = assertLiveProviderEffectOpenState(state, publicationPlan);
  if (terminal === "attempt-fenced") {
    callLiveProviderEffectCheckpoint(testCheckpoint, "before-start-attempt-reproof");
    const secondReproof = observeLiveProviderEffectStartReproof(
      roots,
      admittedArguments,
      publicationPlan,
      effect.source,
      effect.witness.value,
      1,
      assertOwner,
    );
    const attemptEvidence = Object.freeze({
      authority_event_sha256: digest(authorityEvent.bytes),
      attempt_sha256: publicationPlan.planned_start_attempt_sha256,
      invocation_binding_sha256: liveProviderEffectValueDigest(
        publicationPlan.invocation_binding,
      ),
      invocation_limit: 1,
      recovery_invocation_authorized: false,
      replay_authorized: false,
      second_reproof: secondReproof,
      second_reproof_sha256: liveProviderEffectValueDigest(secondReproof),
      variant: "attempt-fence",
    });
    published = publishLiveProviderEffectEvent(
      roots,
      state,
      admittedArguments,
      "start-attempt",
      attemptEvidence,
      assertOwner,
      testCheckpoint,
    );
    const blocked = published.state.liveProviderFixtureEffect;
    if (
      blocked?.currentStage !== "attempt-fenced" ||
      blocked.close !== undefined ||
      blocked.events.at(-1)?.value.event_kind !== "start-attempt"
    ) {
      fail("fixture provider effect attempt fence was not durable", 70);
    }
    return Object.freeze({
      disposition: "attempt-fenced",
      event_sha256: digest(published.event.bytes),
      process_invoked: false,
      receipt_published: false,
    });
  }
  const retirement = retireLiveProviderEffectMarker(
    roots,
    state,
    admittedArguments,
    assertOwner,
    testCheckpoint,
  );
  state = retirement.state;
  effect = assertLiveProviderEffectOpenState(state, publicationPlan);
  const completionEvidence = liveProviderEffectPreAttemptCompletionEvidence(
    effect,
    digest(effect.slot.bytes),
    retirement,
  );
  published = publishLiveProviderEffectEvent(
    roots,
    state,
    admittedArguments,
    "completion",
    completionEvidence,
    assertOwner,
    testCheckpoint,
  );
  state = published.state;
  const completion = published.event;
  let afterCloseLinkFailure;
  const closed = closeMutationLease(
    roots,
    held,
    "completed",
    {
      afterLinkObserver: () => {
        try {
          callLiveProviderEffectCheckpoint(
            testCheckpoint,
            "after-effect-close-link",
          );
        } catch (error) {
          afterCloseLinkFailure = error;
        }
      },
    },
    digest(completion.bytes),
    (current) => {
      const currentEffect = assertLiveProviderEffectOpenState(
        current,
        publicationPlan,
      );
      if (
        currentEffect.currentStage !== "completion-retired" ||
        !currentEffect.completion?.bytes.equals(completion.bytes)
      ) {
        fail("fixture provider effect completion changed", 73);
      }
      assertLiveProviderEffectRetiredWitness(
        current,
        publicationPlan,
        admittedArguments,
      );
    },
  );
  if (afterCloseLinkFailure !== undefined) throw afterCloseLinkFailure;
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-effect-close");
  const durable = closed.liveProviderFixtureEffect;
  if (
    durable?.close?.value.disposition !== "completed" ||
    !durable.completion?.bytes.equals(completion.bytes)
  ) {
    fail("fixture provider effect completion was not durable", 70);
  }
  return durable.completion.value;
}

export function publishColimaLiveProviderEffectPreAttemptForTest(
  argumentsValue,
) {
  return publishColimaLiveProviderFixtureEffect(
    argumentsValue,
    "pre-attempt-retired",
  );
}

export function publishColimaLiveProviderEffectAttemptFenceForTest(
  argumentsValue,
) {
  return publishColimaLiveProviderFixtureEffect(
    argumentsValue,
    "attempt-fenced",
  );
}

function callLiveProviderReservationCheckpoint(testCheckpoint, name) {
  if (
    testCheckpoint !== undefined &&
    testCheckpoint(name) !== undefined
  ) {
    fail("live provider reservation checkpoint returned a value", 70);
  }
}

function liveProviderReservationSourceOperationPlan(source) {
  const operationPlan =
    source.startDecisionSource?.intentPublicationPlan
      ?.provider_operation_plan;
  if (operationPlan === undefined) {
    fail("live provider reservation source plan was unavailable", 69);
  }
  return operationPlan;
}

function validateLiveProviderReservationArgumentBinding(
  argumentsValue,
  source,
  fixtureOnly,
) {
  const operationPlan = liveProviderReservationSourceOperationPlan(source);
  if (
    colimaLiveDigest(colimaLiveBytes(argumentsValue.observation)) !==
      operationPlan.preparation_observation_sha256 ||
    (fixtureOnly &&
      colimaLiveDigest(colimaLiveBytes(argumentsValue.requirements)) !==
        source.startDecisionPublicationPlan.admission.root_observation
          .requirements_sha256)
  ) {
    fail("live provider reservation preparation binding was refused", 73);
  }
}

function reservationDirectoryIdentity(path, expected, label) {
  const metadata = ownedPrivateDirectory(path, label);
  if (
    String(metadata.dev) !== expected.device ||
    String(metadata.ino) !== expected.inode ||
    String(metadata.uid) !== expected.uid ||
    modeString(metadata) !== expected.mode
  ) {
    fail(`${label} identity changed`, 73);
  }
  return metadata;
}

function liveProviderReservationMarkerPath(argumentsValue) {
  return join(
    argumentsValue.observationInput.provider_root,
    COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
  );
}

function liveProviderReservationMarkerIsAbsent(path) {
  try {
    exactLstat(path);
    return false;
  } catch (error) {
    if (error?.code === "ENOENT") return true;
    fail("live provider reservation marker was unavailable", 69);
  }
}

function inspectLiveProviderReservationArtifact(
  path,
  label,
  expectedDevice,
  expectedLinks,
  expectedBytes,
  expectedIdentity = undefined,
) {
  const metadata = inspectPendingFile(
    path,
    label,
    expectedDevice,
    new Set([BigInt(expectedLinks)]),
  );
  if (
    (expectedIdentity !== undefined &&
      !sameMutationArtifact(metadata, expectedIdentity)) ||
    !readPrivate(path, label, expectedLinks, 0n).equals(expectedBytes)
  ) {
    fail(`${label} identity changed`, 73);
  }
  return metadata;
}

function assertLiveProviderReservationBoundDirectories(
  state,
  publicationPlan,
  argumentsValue,
) {
  const runMetadata = reservationDirectoryIdentity(
    state.run,
    publicationPlan.state_run_identity,
    "live provider reservation state run",
  );
  const providerRootMetadata = reservationDirectoryIdentity(
    argumentsValue.observationInput.provider_root,
    publicationPlan.provider_root_identity,
    "live provider reservation provider root",
  );
  if (runMetadata.dev !== providerRootMetadata.dev) {
    fail("live provider reservation filesystem changed", 73);
  }
  return Object.freeze({ providerRootMetadata, runMetadata });
}

function assertLiveProviderReservationMarkerAbsent(
  state,
  publicationPlan,
  argumentsValue,
) {
  assertLiveProviderReservationBoundDirectories(
    state,
    publicationPlan,
    argumentsValue,
  );
  if (
    !liveProviderReservationMarkerIsAbsent(
      liveProviderReservationMarkerPath(argumentsValue),
    )
  ) {
    fail("live provider reservation marker was not absent", 73);
  }
}

function assertLiveProviderReservationLinkedTopology(
  state,
  publicationPlan,
  argumentsValue,
  expectedLinks,
) {
  const { providerRootMetadata, runMetadata } =
    assertLiveProviderReservationBoundDirectories(
      state,
      publicationPlan,
      argumentsValue,
    );
  const artifacts = [
    state.providerReservationStage,
    state.providerReservationWitness,
  ].filter((artifact) => artifact !== undefined);
  if (
    artifacts.length === 0 ||
    artifacts.some(
      (artifact) => artifact.metadata.nlink !== BigInt(expectedLinks),
    )
  ) {
    fail("live provider reservation local topology changed", 73);
  }
  const expectedBytes = artifacts[0].bytes;
  const expectedIdentity = artifacts[0].metadata;
  for (const artifact of artifacts) {
    const metadata = inspectLiveProviderReservationArtifact(
      artifact.path,
      "live provider reservation state artifact",
      runMetadata.dev,
      expectedLinks,
      expectedBytes,
      expectedIdentity,
    );
    if (!sameMutationArtifact(metadata, artifact.metadata)) {
      fail("live provider reservation state artifact changed", 73);
    }
  }
  const marker = inspectLiveProviderReservationArtifact(
    liveProviderReservationMarkerPath(argumentsValue),
    "live provider reservation marker",
    providerRootMetadata.dev,
    expectedLinks,
    expectedBytes,
    expectedIdentity,
  );
  if (!sameMutationArtifact(marker, expectedIdentity)) {
    fail("live provider reservation marker identity changed", 73);
  }
  return Object.freeze({ bytes: expectedBytes, identity: expectedIdentity });
}

function assertLiveProviderReservationRetiredWitness(
  state,
  publicationPlan,
  argumentsValue,
) {
  const { runMetadata } = assertLiveProviderReservationBoundDirectories(
    state,
    publicationPlan,
    argumentsValue,
  );
  const witness = state.providerReservationWitness;
  if (
    state.providerReservationStage !== undefined ||
    witness === undefined ||
    witness.metadata.nlink !== 1n
  ) {
    fail("live provider reservation retirement was not durable", 73);
  }
  inspectLiveProviderReservationArtifact(
    witness.path,
    "live provider reservation witness",
    runMetadata.dev,
    1,
    witness.bytes,
    witness.metadata,
  );
  return witness;
}

function observeLiveProviderReservationPhysicalRoots(
  argumentsValue,
  state,
  source,
  publicationPlan,
  fixtureOnly,
  markerDisposition,
  testCheckpoint,
) {
  validateLiveProviderReservationArgumentBinding(
    argumentsValue,
    source,
    fixtureOnly,
  );
  let observation;
  try {
    observation = fixtureOnly
      ? observeColimaLiveProviderReservationRootsForTest(
          argumentsValue.requirements,
          argumentsValue.observation,
          argumentsValue.observationInput,
          state.run,
          markerDisposition,
          testCheckpoint,
        )
      : observeColimaLiveProviderReservationRoots(
          argumentsValue.observation,
          argumentsValue.observationInput,
          state.run,
          markerDisposition,
        );
  } catch (error) {
    if (error instanceof ColimaLiveContractFailure) {
      fail("live provider reservation root observation was refused", error.exitStatus);
    }
    throw error;
  }
  if (
    publicationPlan !== undefined &&
    (observation.marker_disposition !== markerDisposition ||
      !liveProviderReservationBytes(observation.root_observation).equals(
        liveProviderReservationBytes(
          publicationPlan.admission.root_observation,
        ),
      ) ||
      !liveProviderReservationBytes(observation.provider_root_identity).equals(
        liveProviderReservationBytes(publicationPlan.provider_root_identity),
      ) ||
      !liveProviderReservationBytes(observation.state_run_identity).equals(
        liveProviderReservationBytes(publicationPlan.state_run_identity),
      ) ||
      !liveProviderReservationBytes(observation.namespace_bindings).equals(
        liveProviderReservationBytes(publicationPlan.namespace_bindings),
      ))
  ) {
    fail("live provider reservation root observation changed", 73);
  }
  return observation;
}

function completedLiveProviderReservationForVariant(state, fixtureOnly) {
  const own = fixtureOnly
    ? state.liveProviderFixtureReservation
    : state.liveProviderReservation;
  const other = fixtureOnly
    ? state.liveProviderReservation
    : state.liveProviderFixtureReservation;
  if (other !== undefined) {
    fail("live provider reservation evidence class differed", 73);
  }
  return own;
}

function assertLiveProviderReservationOpenState(
  state,
  fixtureOnly,
  publicationPlan,
) {
  const reservation = completedLiveProviderReservationForVariant(
    state,
    fixtureOnly,
  );
  const expectedContract = fixtureOnly
    ? "live-provider-fixture-reservation-only"
    : "live-provider-reservation-only";
  if (
    reservation === undefined ||
    reservation.close !== undefined ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(reservation.slot.bytes) ||
    !liveProviderReservationBytes(reservation.publicationPlan).equals(
      liveProviderReservationBytes(publicationPlan),
    ) ||
    state.receiptState.head.sequence !== reservation.slot.value.source_sequence ||
    state.receiptState.head_sha256 !== reservation.slot.value.source_head_sha256 ||
    state.environment !== undefined ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined ||
    state.providerState.contract !== expectedContract ||
    state.providerState.operationEvidenceSha256 !== ZERO_SHA256 ||
    state.cleanupState.contract !== "journal-only" ||
    state.cleanupState.operationEvidenceSha256 !== ZERO_SHA256
  ) {
    fail("live provider reservation state changed", 73);
  }
  return reservation;
}

function liveProviderReservationCompletion(reservation) {
  if (
    reservation.close?.value.disposition !== "completed" ||
    reservation.settlement === undefined ||
    reservation.witness === undefined
  ) {
    fail("completed live provider reservation was unavailable", 69);
  }
  try {
    return buildColimaLiveProviderReservationCompletion({
      closeSha256: digest(reservation.close.bytes),
      publicationPlan: reservation.publicationPlan,
      settlement: reservation.settlement.value,
      slotSequence: reservation.slot.value.journal_sequence,
      slotSha256: digest(reservation.slot.bytes),
      source: reservation.source,
      witness: reservation.witness.value,
    });
  } catch (error) {
    if (error instanceof LiveProviderReservationFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function buildLiveProviderReservationPublicationPlan(
  argumentsValue,
  state,
  snapshot,
  admission,
  fixtureOnly,
  testCheckpoint,
) {
  const observed = observeLiveProviderReservationPhysicalRoots(
    argumentsValue,
    state,
    snapshot.source,
    undefined,
    fixtureOnly,
    "absent",
    testCheckpoint,
  );
  if (
    !liveProviderReservationBytes(observed.root_observation).equals(
      liveProviderReservationBytes(admission.root_observation),
    )
  ) {
    fail("live provider reservation admission changed", 73);
  }
  try {
    return buildColimaLiveProviderReservationPublicationPlan({
      admission,
      namespaceBindings: observed.namespace_bindings,
      providerRootIdentity: observed.provider_root_identity,
      source: snapshot.source,
      stateRunIdentity: observed.state_run_identity,
    });
  } catch (error) {
    if (error instanceof LiveProviderReservationFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function completedLiveProviderStartDecisionReservationSnapshot(
  state,
  fixtureOnly,
) {
  const completed = fixtureOnly
    ? state.liveProviderFixtureStartDecision
    : state.liveProviderStartDecision;
  const other = fixtureOnly
    ? state.liveProviderStartDecision
    : state.liveProviderFixtureStartDecision;
  if (completed === undefined || other !== undefined) {
    fail("completed live provider start decision was unavailable", 69);
  }
  const decisionSequence = completed.slot.value.journal_sequence;
  const closes = new Map(
    state.mutationCloses.map((close) => [close.value.slot_sequence, close]),
  );
  const reservationTail = state.mutationSlots.slice(decisionSequence + 1);
  if (
    reservationTail.some(
      (slot) =>
        slot.value.action !== COLIMA_LIVE_PROVIDER_RESERVATION_ACTION ||
        liveProviderReservationFixtureOnly(
          slot.value.operation_kind,
          slot.value.operation_contract_sha256,
        ) !== fixtureOnly ||
        closes.get(slot.value.journal_sequence)?.value.disposition !==
          "aborted-before-effect",
    ) ||
    state.mutationLease !== undefined ||
    state.mutationRecoveries.length !== 0 ||
    state.providerReservationStage !== undefined ||
    state.providerReservationWitness !== undefined ||
    state.mutationOperations.length !== 0
  ) {
    fail("completed live provider start decision was unavailable", 69);
  }
  const historicalState = {
    ...state,
    liveProviderFixtureReservation: undefined,
    liveProviderReservation: undefined,
    mutationCloses: state.mutationCloses.filter(
      (close) => close.value.slot_sequence <= decisionSequence,
    ),
    mutationSlots: state.mutationSlots.slice(0, decisionSequence + 1),
    providerState: Object.freeze({
      contract: fixtureOnly
        ? "live-provider-fixture-start-decision-only"
        : "live-provider-start-decision-only",
      operationEvidenceSha256: ZERO_SHA256,
    }),
  };
  return completedLiveProviderStartDecisionCoreSnapshot(
    historicalState,
    fixtureOnly,
  );
}

function availableLiveProviderReservationStartDecisionSnapshot(
  state,
  publicationPlan,
  ownStage,
  fixtureOnly,
) {
  if (
    state.mutationStages.some(
      (stage) => stage.linkedDestination !== undefined,
    )
  ) {
    fail("live provider reservation slot stage was refused", 73);
  }
  if (ownStage === undefined) {
    if (state.mutationStages.length !== 0) {
      fail("live provider reservation slot stage was refused", 73);
    }
  } else {
    const stage = state.mutationStages.find(
      (candidate) => candidate.path === ownStage.stagePath,
    );
    if (
      stage === undefined ||
      stage.name !== ownStage.stagePath.split(sep).at(-1) ||
      !sameMetadata(stage.metadata, ownStage.identity)
    ) {
      fail("live provider reservation slot stage changed", 73);
    }
    const staged = parseCanonical(
      ownStage.stagePath,
      "prospective live provider reservation slot",
      1,
    );
    validateMutationLeaseValue(staged.value, state.candidate.run_id);
    const previousClose = state.mutationCloses.at(-1);
    if (
      ownStage.destinationName !==
        mutationSlotFileName(state.mutationSlots.length) ||
      staged.value.action !== COLIMA_LIVE_PROVIDER_RESERVATION_ACTION ||
      staged.value.journal_sequence !== state.mutationSlots.length ||
      staged.value.source_sequence !== 0 ||
      staged.value.source_head_sha256 !== state.receiptState.head_sha256 ||
      staged.value.source_environment_sha256 !== ZERO_SHA256 ||
      staged.value.intent_receipt_sha256 !== ZERO_SHA256 ||
      staged.value.previous_close_sha256 !== digest(previousClose.bytes) ||
      staged.value.operation_kind !== publicationPlan.operation_kind ||
      staged.value.operation_contract_sha256 !==
        publicationPlan.operation_contract_sha256 ||
      !liveProviderReservationBytes(staged.value.operation_plan).equals(
        liveProviderReservationBytes(publicationPlan),
      ) ||
      mutationOwnerState(staged.value) !== "current"
    ) {
      fail("prospective live provider reservation slot was refused", 73);
    }
  }
  return completedLiveProviderStartDecisionReservationSnapshot(
    { ...state, mutationStages: [] },
    fixtureOnly,
  );
}

function buildLiveProviderReservationWitness(
  publicationPlan,
  held,
  source,
) {
  try {
    return buildColimaLiveProviderReservationWitness({
      publicationPlan,
      slotSequence: held.lease.journal_sequence,
      slotSha256: digest(held.leaseBytes),
      source,
    });
  } catch (error) {
    if (error instanceof LiveProviderReservationFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function writeLiveProviderReservationStage(
  state,
  witness,
  testCheckpoint,
) {
  const bytes = liveProviderReservationBytes(witness);
  const path = join(
    state.run,
    `${PROVIDER_RESERVATION_STAGE_PREFIX}${randomBytes(16).toString("hex")}`,
  );
  writeExclusive(path, bytes);
  const runMetadata = ownedPrivateDirectory(state.run, "active run state");
  const identity = inspectLiveProviderReservationArtifact(
    path,
    "live provider reservation stage",
    runMetadata.dev,
    1,
    bytes,
  );
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-witness-stage");
  return Object.freeze({ bytes, identity, path });
}

function linkLiveProviderReservationMarker(
  state,
  stage,
  argumentsValue,
  testCheckpoint,
) {
  const markerPath = liveProviderReservationMarkerPath(argumentsValue);
  try {
    linkSync(stage.path, markerPath);
  } catch (error) {
    if (error?.code === "EEXIST") return false;
    fail("live provider reservation marker publication failed", 70);
  }
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-marker-link");
  syncDirectory(argumentsValue.observationInput.provider_root);
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-marker-directory-sync",
  );
  return true;
}

function linkLiveProviderReservationWitness(
  state,
  stage,
  witnessPath,
  testCheckpoint,
) {
  try {
    linkSync(stage.path, witnessPath);
  } catch {
    fail("live provider reservation witness publication failed", 70);
  }
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-state-witness-link");
  syncDirectory(state.run);
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-state-witness-directory-sync",
  );
}

function unlinkLiveProviderReservationStage(
  state,
  stage,
  expectedLinks,
  testCheckpoint,
) {
  const runMetadata = ownedPrivateDirectory(state.run, "active run state");
  inspectLiveProviderReservationArtifact(
    stage.path,
    "live provider reservation stage",
    runMetadata.dev,
    expectedLinks,
    stage.bytes,
    stage.identity ?? stage.metadata,
  );
  try {
    unlinkSync(stage.path);
  } catch {
    fail("live provider reservation stage retirement failed", 70);
  }
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-witness-stage-unlink");
  syncDirectory(state.run);
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-witness-stage-directory-sync",
  );
}

function publishLiveProviderReservationSettlement(
  roots,
  state,
  reservation,
  witness,
  preRetirementRootObservation,
  reassertAuthority,
  testCheckpoint,
) {
  if (
    typeof reassertAuthority !== "function" ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(reservation.slot.bytes) ||
    reservation.settlement !== undefined ||
    reservation.witness === undefined ||
    !liveProviderReservationBytes(reservation.witness.value).equals(
      liveProviderReservationBytes(witness),
    )
  ) {
    fail("live provider reservation settlement authority was refused", 73);
  }
  const recovery = state.mutationRecoveries.at(-1);
  let settlement;
  try {
    settlement = buildColimaLiveProviderReservationSettlement({
      authority: recovery === undefined ? "owner" : "recovery",
      authoritySha256:
        recovery === undefined
          ? digest(reservation.slot.bytes)
          : digest(recovery.bytes),
      preRetirementRootObservation,
      publicationPlan: reservation.publicationPlan,
      slotSequence: reservation.slot.value.journal_sequence,
      slotSha256: digest(reservation.slot.bytes),
      source: reservation.source,
      witness,
    });
  } catch (error) {
    if (error instanceof LiveProviderReservationFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
  const bytes = liveProviderReservationBytes(settlement);
  const name = mutationOperationFileName(
    reservation.slot.value.journal_sequence,
  );
  let afterLinkFailure;
  const reassertSettlementPublication = () => {
    const current = reassertAuthority();
    const currentReservation =
      current.liveProviderReservation ??
      current.liveProviderFixtureReservation;
    if (
      currentReservation === undefined ||
      currentReservation.settlement !== undefined ||
      currentReservation.witness === undefined ||
      !currentReservation.slot.bytes.equals(reservation.slot.bytes) ||
      !currentReservation.witness.bytes.equals(reservation.witness.bytes)
    ) {
      fail("live provider reservation settlement source changed", 73);
    }
  };
  if (!publishMutationBlocker(state.run, name, bytes, {
    afterLinkObserver:
      testCheckpoint === undefined
        ? undefined
        : () => {
            try {
              callLiveProviderReservationCheckpoint(
                testCheckpoint,
                "after-retirement-authorization-link",
              );
            } catch (error) {
              afterLinkFailure = error;
            }
          },
    reassertAuthority: reassertSettlementPublication,
  })) {
    const existing = parseCanonical(
      join(state.run, name),
      "live provider reservation settlement",
    );
    if (!existing.bytes.equals(bytes)) {
      fail("live provider reservation settlement publication conflicted", 73);
    }
  }
  const verified = loadState(roots, false);
  const verifiedReservation =
    verified.liveProviderReservation ??
    verified.liveProviderFixtureReservation;
  if (
    verifiedReservation?.settlement === undefined ||
    !verifiedReservation.settlement.bytes.equals(bytes)
  ) {
    fail("live provider reservation settlement was not durable", 70);
  }
  if (afterLinkFailure !== undefined) throw afterLinkFailure;
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-retirement-authorization-publication",
  );
  return verified;
}

function retireLiveProviderReservationMarker(
  roots,
  state,
  publicationPlan,
  argumentsValue,
  testCheckpoint,
) {
  const reservation =
    state.liveProviderReservation ?? state.liveProviderFixtureReservation;
  if (
    reservation?.settlement === undefined ||
    reservation.witness === undefined
  ) {
    fail("live provider reservation retirement authority was refused", 73);
  }
  const held = assertLiveProviderReservationLinkedTopology(
    state,
    publicationPlan,
    argumentsValue,
    2,
  );
  const markerPath = liveProviderReservationMarkerPath(argumentsValue);
  inspectLiveProviderReservationArtifact(
    markerPath,
    "live provider reservation marker",
    BigInt(publicationPlan.provider_root_identity.device),
    2,
    held.bytes,
    held.identity,
  );
  try {
    unlinkSync(markerPath);
  } catch {
    fail("live provider reservation marker retirement failed", 70);
  }
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-marker-unlink");
  syncDirectory(argumentsValue.observationInput.provider_root);
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-marker-retirement-directory-sync",
  );
  const verified = loadState(roots, false);
  const witness = assertLiveProviderReservationRetiredWitness(
    verified,
    publicationPlan,
    argumentsValue,
  );
  if (
    witness.metadata.nlink !== 1n ||
    !witness.bytes.equals(reservation.witness.bytes)
  ) {
    fail("live provider reservation marker retirement was not durable", 70);
  }
  return verified;
}

function reservationAdmissionArguments(argumentsValue, fixtureOnly) {
  if (!fixtureOnly) return admissionArguments(argumentsValue);
  const admitted = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admitted.requirements === null ||
    Array.isArray(admitted.requirements) ||
    typeof admitted.requirements !== "object" ||
    typeof admitted.testCheckpoint !== "function"
  ) {
    fail("live provider fixture reservation arguments were refused", 64);
  }
  return admitted;
}

function observeLiveProviderReservationStartAdmission(
  argumentsValue,
  fixtureOnly,
) {
  return observeColimaLiveProviderStartEffectFreshAdmission(
    argumentsValue,
    {
      fixtureOnly,
      stateSnapshot: (state) =>
        completedLiveProviderStartDecisionReservationSnapshot(
          state,
          fixtureOnly,
        ),
      testCheckpoint: fixtureOnly
        ? argumentsValue.testCheckpoint
        : undefined,
    },
  );
}

function publishColimaLiveProviderReservation(
  argumentsValue,
  fixtureOnly,
) {
  const admittedArguments = reservationAdmissionArguments(
    argumentsValue,
    fixtureOnly,
  );
  const testCheckpoint = fixtureOnly
    ? admittedArguments.testCheckpoint
    : undefined;
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  reconcileMutationStages(roots);
  const initial = loadState(roots, true);
  const completed = completedLiveProviderReservationForVariant(
    initial,
    fixtureOnly,
  );
  if (completed?.close?.value.disposition === "completed") {
    validateLiveProviderReservationArgumentBinding(
      admittedArguments,
      completed.source,
      fixtureOnly,
    );
    assertLiveProviderReservationRetiredWitness(
      initial,
      completed.publicationPlan,
      admittedArguments,
    );
    return liveProviderReservationCompletion(completed);
  }
  const snapshot = completedLiveProviderStartDecisionReservationSnapshot(
    initial,
    fixtureOnly,
  );
  const startAdmission = observeLiveProviderReservationStartAdmission(
    admittedArguments,
    fixtureOnly,
  );
  const publicationPlan = buildLiveProviderReservationPublicationPlan(
    admittedArguments,
    initial,
    snapshot,
    startAdmission,
    fixtureOnly,
    undefined,
  );
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-reservation-plan");
  const reassertSlotPublication = (ownStage) => {
    const current = loadState(roots, true);
    const currentSnapshot =
      availableLiveProviderReservationStartDecisionSnapshot(
        current,
        publicationPlan,
        ownStage,
        fixtureOnly,
      );
    if (
      !sameCompletedLiveProviderStartDecisionSnapshot(
        snapshot,
        currentSnapshot,
      )
    ) {
      fail("live provider reservation start decision changed", 73);
    }
    assertLiveProviderReservationMarkerAbsent(
      current,
      publicationPlan,
      admittedArguments,
    );
    observeLiveProviderReservationPhysicalRoots(
      admittedArguments,
      current,
      snapshot.source,
      publicationPlan,
      fixtureOnly,
      "absent",
      undefined,
    );
  };
  const expectedSource = Object.freeze({
    sequence: initial.receiptState.head.sequence,
    sha256: initial.receiptState.head_sha256,
  });
  let afterSlotLinkFailure;
  const held = acquireMutationLease(
    roots,
    COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
    ZERO_SHA256,
    expectedSource,
    {
      afterLinkObserver:
        testCheckpoint === undefined
          ? undefined
          : () => {
              try {
                callLiveProviderReservationCheckpoint(
                  testCheckpoint,
                  "after-reservation-slot-link",
                );
              } catch (error) {
                afterSlotLinkFailure = error;
              }
            },
      reassertAuthority: reassertSlotPublication,
    },
    Object.freeze({
      contractSha256: publicationPlan.operation_contract_sha256,
      kind: publicationPlan.operation_kind,
      plan: publicationPlan,
    }),
  );
  if (afterSlotLinkFailure !== undefined) throw afterSlotLinkFailure;
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-reservation-slot");
  let state = assertMutationLeaseHeld(roots, held, false);
  let reservation = assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    publicationPlan,
  );
  assertLiveProviderReservationMarkerAbsent(
    state,
    publicationPlan,
    admittedArguments,
  );
  observeLiveProviderReservationPhysicalRoots(
    admittedArguments,
    state,
    reservation.source,
    publicationPlan,
    fixtureOnly,
    "absent",
    undefined,
  );
  const witness = buildLiveProviderReservationWitness(
    publicationPlan,
    held,
    reservation.source,
  );
  const stage = writeLiveProviderReservationStage(
    state,
    witness,
    testCheckpoint,
  );
  state = assertMutationLeaseHeld(roots, held, false);
  reservation = assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    publicationPlan,
  );
  if (
    !linkLiveProviderReservationMarker(
      state,
      stage,
      admittedArguments,
      testCheckpoint,
    )
  ) {
    unlinkLiveProviderReservationStage(state, stage, 1, testCheckpoint);
    closeMutationLease(
      roots,
      held,
      "aborted-before-effect",
      {},
      ZERO_SHA256,
      (current) => {
        assertLiveProviderReservationOpenState(
          current,
          fixtureOnly,
          publicationPlan,
        );
      },
    );
    fail("live provider reservation marker was already held", 73);
  }
  state = assertMutationLeaseHeld(roots, held, false);
  assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    publicationPlan,
  );
  assertLiveProviderReservationLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    2,
  );
  const witnessPath = join(
    state.run,
    providerReservationWitnessFileName(held.lease.journal_sequence),
  );
  linkLiveProviderReservationWitness(
    state,
    stage,
    witnessPath,
    testCheckpoint,
  );
  state = assertMutationLeaseHeld(roots, held, false);
  assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    publicationPlan,
  );
  assertLiveProviderReservationLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    3,
  );
  unlinkLiveProviderReservationStage(state, stage, 3, testCheckpoint);
  state = assertMutationLeaseHeld(roots, held, false);
  reservation = assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    publicationPlan,
  );
  assertLiveProviderReservationLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    2,
  );
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "before-retirement-observation",
  );
  assertLiveProviderReservationLinkedTopology(
    state,
    publicationPlan,
    admittedArguments,
    2,
  );
  const observed = observeLiveProviderReservationPhysicalRoots(
    admittedArguments,
    state,
    reservation.source,
    publicationPlan,
    fixtureOnly,
    "present",
    undefined,
  );
  assertLiveProviderReservationLinkedTopology(
    loadState(roots, true),
    publicationPlan,
    admittedArguments,
    2,
  );
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-retirement-observation",
  );
  const ownerAuthority = () => {
    const current = assertMutationLeaseHeld(roots, held, false);
    const currentReservation = assertLiveProviderReservationOpenState(
      current,
      fixtureOnly,
      publicationPlan,
    );
    if (currentReservation.settlement !== undefined) {
      fail("live provider reservation settlement already existed", 73);
    }
    assertLiveProviderReservationLinkedTopology(
      current,
      publicationPlan,
      admittedArguments,
      2,
    );
    observeLiveProviderReservationPhysicalRoots(
      admittedArguments,
      current,
      currentReservation.source,
      publicationPlan,
      fixtureOnly,
      "present",
      undefined,
    );
    assertLiveProviderReservationLinkedTopology(
      loadState(roots, true),
      publicationPlan,
      admittedArguments,
      2,
    );
    return current;
  };
  state = publishLiveProviderReservationSettlement(
    roots,
    state,
    reservation,
    witness,
    observed.root_observation,
    ownerAuthority,
    testCheckpoint,
  );
  retireLiveProviderReservationMarker(
    roots,
    state,
    publicationPlan,
    admittedArguments,
    testCheckpoint,
  );
  state = assertMutationLeaseHeld(roots, held, false);
  reservation = assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    publicationPlan,
  );
  const retiredWitness = assertLiveProviderReservationRetiredWitness(
    state,
    publicationPlan,
    admittedArguments,
  );
  if (reservation.settlement === undefined) {
    fail("live provider reservation settlement was unavailable", 70);
  }
  let afterCloseLinkFailure;
  const closed = closeMutationLease(
    roots,
    held,
    "completed",
    {
      afterLinkObserver:
        testCheckpoint === undefined
          ? undefined
          : () => {
              try {
                callLiveProviderReservationCheckpoint(
                  testCheckpoint,
                  "after-reservation-close-link",
                );
              } catch (error) {
                afterCloseLinkFailure = error;
              }
            },
    },
    digest(reservation.settlement.bytes),
    (current) => {
      const currentReservation = assertLiveProviderReservationOpenState(
        current,
        fixtureOnly,
        publicationPlan,
      );
      if (
        currentReservation.settlement === undefined ||
        !currentReservation.settlement.bytes.equals(
          reservation.settlement.bytes,
        )
      ) {
        fail("live provider reservation settlement changed", 73);
      }
      assertLiveProviderReservationRetiredWitness(
        current,
        publicationPlan,
        admittedArguments,
      );
    },
  );
  if (afterCloseLinkFailure !== undefined) throw afterCloseLinkFailure;
  callLiveProviderReservationCheckpoint(testCheckpoint, "after-reservation-close");
  const completedReservation = completedLiveProviderReservationForVariant(
    closed,
    fixtureOnly,
  );
  if (
    completedReservation === undefined ||
    completedReservation.witness?.path !== retiredWitness.path
  ) {
    fail("live provider reservation completion was not durable", 70);
  }
  return liveProviderReservationCompletion(completedReservation);
}

export function publishColimaLiveProviderReservationForExecutor(
  argumentsValue,
) {
  return publishColimaLiveProviderReservation(argumentsValue, false);
}

export function publishColimaLiveProviderReservationForTest(argumentsValue) {
  return publishColimaLiveProviderReservation(argumentsValue, true);
}

function validateHistoricalLiveProviderStartDecisionRetry(
  argumentsValue,
  completed,
  fixtureOnly,
) {
  const operationPlan =
    completed.source.intentPublicationPlan.provider_operation_plan;
  if (
    colimaLiveDigest(colimaLiveBytes(argumentsValue.observation)) !==
      operationPlan.preparation_observation_sha256 ||
    (fixtureOnly &&
      colimaLiveDigest(colimaLiveBytes(argumentsValue.requirements)) !==
        completed.publicationPlan.admission.root_observation.requirements_sha256)
  ) {
    fail("completed live provider start decision retry differed", 73);
  }
  return liveProviderStartDecisionCompletion(completed);
}

function publishColimaLiveProviderStartDecision(
  admittedArguments,
  { fixtureOnly, testCheckpoint },
) {
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  reconcileMutationStages(roots);
  const initial = loadState(roots, true);
  const completed = completedLiveProviderStartDecisionForVariant(
    initial,
    fixtureOnly,
  );
  if (completed !== undefined) {
    return validateHistoricalLiveProviderStartDecisionRetry(
      admittedArguments,
      completed,
      fixtureOnly,
    );
  }
  assertLiveProviderStartDecisionVariantHistory(initial, fixtureOnly);
  const baseline = observeLiveProviderStartDecisionAdmission(
    admittedArguments,
    {
      boundary: "initial",
      fixtureOnly,
      stateSnapshot: (state) =>
        completedLiveProviderIntentSnapshot(state, fixtureOnly),
      testCheckpoint,
    },
  );
  assertLiveProviderStartDecisionAdmissionPristine(baseline.admission);
  const publicationPlan = buildLiveProviderStartDecisionPublicationPlan(
    baseline.admission,
    baseline.snapshot,
  );
  testCheckpoint?.("after-initial-admission");
  const reassertSlotPublication = (ownStage) => {
    const current = observeLiveProviderStartDecisionAdmission(
      admittedArguments,
      {
        boundary: "before-slot-link",
        fixtureOnly,
        stateSnapshot: (state) =>
          availableLiveProviderStartDecisionSnapshot(
            state,
            publicationPlan,
            ownStage,
            fixtureOnly,
          ),
        testCheckpoint,
      },
    );
    assertLiveProviderStartDecisionAdmissionPristine(current.admission);
    const rebuilt = buildLiveProviderStartDecisionPublicationPlan(
      current.admission,
      current.snapshot,
    );
    if (
      !sameLiveProviderStartDecisionValue(
        current.admission,
        baseline.admission,
      ) ||
      !sameLiveProviderStartDecisionValue(rebuilt, publicationPlan)
    ) {
      fail(
        "live provider start decision observation changed before slot publication",
        73,
      );
    }
  };
  const expectedSource = Object.freeze({
    sequence: initial.receiptState.head.sequence,
    sha256: initial.receiptState.head_sha256,
  });
  let afterSlotLinkFailure;
  const held = acquireMutationLease(
    roots,
    COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
    ZERO_SHA256,
    expectedSource,
    {
      afterAuthorityObserver:
        testCheckpoint === undefined
          ? undefined
          : () => testCheckpoint("after-slot-authority-reassertion"),
      afterLinkObserver:
        testCheckpoint === undefined
          ? undefined
          : () => {
              try {
                testCheckpoint("after-slot-link");
              } catch (error) {
                afterSlotLinkFailure = error;
              }
            },
      beforeStageObserver:
        testCheckpoint === undefined
          ? undefined
          : () => testCheckpoint("before-slot-stage"),
      reassertAuthority: reassertSlotPublication,
    },
    Object.freeze({
      contractSha256: publicationPlan.operation_contract_sha256,
      kind: publicationPlan.operation_kind,
      plan: publicationPlan,
    }),
  );
  let publicationFailure;
  try {
    if (afterSlotLinkFailure !== undefined) throw afterSlotLinkFailure;
    const acquired = observeLiveProviderStartDecisionAdmission(
      admittedArguments,
      {
        boundary: "after-slot-acquisition",
        fixtureOnly,
        stateSnapshot: (state) =>
          heldLiveProviderStartDecisionSnapshot(
            state,
            held,
            publicationPlan,
            undefined,
            fixtureOnly,
          ),
        testCheckpoint,
      },
    );
    assertLiveProviderStartDecisionAdmissionPristine(acquired.admission);
    const acquiredPlan = buildLiveProviderStartDecisionPublicationPlan(
      acquired.admission,
      acquired.snapshot,
    );
    if (
      !sameLiveProviderStartDecisionValue(
        acquired.admission,
        baseline.admission,
      ) ||
      !sameLiveProviderStartDecisionValue(acquiredPlan, publicationPlan)
    ) {
      fail(
        "live provider start decision observation changed after slot acquisition",
        73,
      );
    }
    testCheckpoint?.("after-slot-acquisition");
    let afterCloseLinkFailure;
    closeMutationLease(
      roots,
      held,
      "completed",
      {
        afterLinkObserver:
          testCheckpoint === undefined
            ? undefined
            : () => {
                try {
                  testCheckpoint("after-close-link");
                } catch (error) {
                  afterCloseLinkFailure = error;
                }
              },
      },
      ZERO_SHA256,
      (state, ownStage) => {
        if (ownStage === undefined) return;
        const closing = observeLiveProviderStartDecisionAdmission(
          admittedArguments,
          {
            boundary: "before-close-link",
            fixtureOnly,
            stateSnapshot: (current) =>
              heldLiveProviderStartDecisionSnapshot(
                current,
                held,
                publicationPlan,
                ownStage,
                fixtureOnly,
              ),
            testCheckpoint,
          },
        );
        assertLiveProviderStartDecisionAdmissionPristine(closing.admission);
        const closingPlan = buildLiveProviderStartDecisionPublicationPlan(
          closing.admission,
          closing.snapshot,
        );
        if (
          !sameLiveProviderStartDecisionValue(
            closing.admission,
            baseline.admission,
          ) ||
          !sameLiveProviderStartDecisionValue(
            closing.admission,
            acquired.admission,
          ) ||
          !sameLiveProviderStartDecisionValue(closingPlan, publicationPlan) ||
          state.mutationLease === undefined ||
          !state.mutationLease.bytes.equals(held.leaseBytes)
        ) {
          fail(
            "live provider start decision observation changed before close publication",
            73,
          );
        }
      },
    );
    if (afterCloseLinkFailure !== undefined) throw afterCloseLinkFailure;
  } catch (error) {
    publicationFailure = error;
  }
  if (publicationFailure !== undefined) {
    const state = loadState(roots, false);
    if (
      state.mutationLease !== undefined &&
      state.mutationLease.bytes.equals(held.leaseBytes)
    ) {
      closeMutationLease(roots, held, "aborted-before-effect");
    }
    throw publicationFailure;
  }
  const verified = loadState(roots, true);
  const recorded = completedLiveProviderStartDecisionForVariant(
    verified,
    fixtureOnly,
  );
  if (
    recorded === undefined ||
    !canonicalBytes(recorded.publicationPlan).equals(
      canonicalBytes(publicationPlan),
    )
  ) {
    fail("live provider start decision publication was not durable", 70);
  }
  return liveProviderStartDecisionCompletion(recorded);
}

export function publishColimaLiveProviderStartDecisionForExecutor(
  argumentsValue,
) {
  return publishColimaLiveProviderStartDecision(
    admissionArguments(argumentsValue),
    { fixtureOnly: false, testCheckpoint: undefined },
  );
}

// This unsupported fixture seam substitutes only the bounded root requirements
// and checkpoints. It persists a separate fixture operation identity.
export function publishColimaLiveProviderStartDecisionForTest(argumentsValue) {
  const admittedArguments = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admittedArguments.requirements === null ||
    Array.isArray(admittedArguments.requirements) ||
    typeof admittedArguments.requirements !== "object" ||
    typeof admittedArguments.testCheckpoint !== "function"
  ) {
    fail("live provider fixture start decision arguments were refused", 64);
  }
  return publishColimaLiveProviderStartDecision(admittedArguments, {
    fixtureOnly: true,
    testCheckpoint: admittedArguments.testCheckpoint,
  });
}

function observeLiveProviderIntentAdmission(
  argumentsValue,
  { boundary, fixtureOnly, stateSnapshot, testCheckpoint },
) {
  const checkpoint =
    testCheckpoint === undefined
      ? undefined
      : (name) => testCheckpoint(`${boundary}:${name}`);
  return observeColimaLivePreEffectAdmission(
    argumentsValue,
    {
      admissionSchema: fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA
        : COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
      authority: fixtureOnly
        ? "fixture-only-point-in-time-not-effect-authority"
        : "point-in-time-not-effect-authority",
      fixtureOnly,
      stateSnapshot,
      testCheckpoint: checkpoint,
    },
  );
}

function buildLiveProviderIntentPublicationPlan(
  admission,
  fixtureOnly,
  operationPlan,
) {
  try {
    return buildColimaLiveProviderIntentPublicationPlan({
      admission,
      fixtureOnly,
      operationPlan,
    });
  } catch (error) {
    if (error instanceof LiveProviderIntentFailure) {
      fail(error.message, error.exitStatus);
    }
    throw error;
  }
}

function sameLiveProviderIntentValue(left, right) {
  return liveProviderIntentBytes(left).equals(liveProviderIntentBytes(right));
}

function assertLiveProviderIntentAdmissionPristine(admission) {
  if (
    admission.root_observation.root_set_disposition !== "observed-pristine" ||
    admission.intent_candidate === null ||
    admission.pre_effect_prefix === null
  ) {
    fail("live provider intent namespaces were not pristine", 73);
  }
  return admission;
}

function completedLiveProviderIntentForVariant(state, fixtureOnly) {
  const own = fixtureOnly
    ? state.liveProviderFixtureIntent
    : state.liveProviderIntent;
  const other = fixtureOnly
    ? state.liveProviderIntent
    : state.liveProviderFixtureIntent;
  if (other !== undefined) {
    fail("live provider intent evidence class already differs", 73);
  }
  return own;
}

function validateHistoricalLiveProviderIntentRetry(
  argumentsValue,
  completed,
  fixtureOnly,
) {
  const operationPlan = completed.publicationPlan.provider_operation_plan;
  if (
    colimaLiveDigest(colimaLiveBytes(argumentsValue.observation)) !==
      operationPlan.preparation_observation_sha256 ||
    (fixtureOnly &&
      colimaLiveDigest(colimaLiveBytes(argumentsValue.requirements)) !==
        completed.publicationPlan.admission.root_observation.requirements_sha256)
  ) {
    fail("completed live provider intent retry differed", 73);
  }
  return liveProviderIntentCompletion(completed);
}

function assertLiveProviderIntentVariantHistory(state, fixtureOnly) {
  for (const slot of state.mutationSlots) {
    if (slot.value.action !== COLIMA_LIVE_PROVIDER_INTENT_ACTION) continue;
    if (
      liveProviderIntentFixtureOnly(
        slot.value.operation_kind,
        slot.value.operation_contract_sha256,
      ) !== fixtureOnly
    ) {
      fail("live provider intent evidence classes were mixed", 73);
    }
  }
}

function publishColimaLiveProviderIntent(
  admittedArguments,
  { fixtureOnly, testCheckpoint },
) {
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  reconcileMutationStages(roots);
  const initial = loadState(roots, true);
  const completed = completedLiveProviderIntentForVariant(
    initial,
    fixtureOnly,
  );
  if (completed !== undefined) {
    return validateHistoricalLiveProviderIntentRetry(
      admittedArguments,
      completed,
      fixtureOnly,
    );
  }
  assertLiveProviderIntentVariantHistory(initial, fixtureOnly);
  const baselineAdmission = observeLiveProviderIntentAdmission(
    admittedArguments,
    {
      boundary: "initial",
      fixtureOnly,
      stateSnapshot: completedLiveProviderPlanSnapshot,
      testCheckpoint,
    },
  );
  assertLiveProviderIntentAdmissionPristine(baselineAdmission);
  const operationPlan = completedLiveProviderPlanSnapshot(
    loadState(roots, true),
  ).operationPlan;
  const publicationPlan = buildLiveProviderIntentPublicationPlan(
    baselineAdmission,
    fixtureOnly,
    operationPlan,
  );
  testCheckpoint?.("after-initial-admission");
  const reassertSlotPublication = (ownStage) => {
    const currentAdmission = observeLiveProviderIntentAdmission(
      admittedArguments,
      {
        boundary: "before-slot-link",
        fixtureOnly,
        stateSnapshot: (state) =>
          availableLiveProviderIntentPlanSnapshot(
            state,
            publicationPlan,
            ownStage,
            fixtureOnly,
          ),
        testCheckpoint,
      },
    );
    assertLiveProviderIntentAdmissionPristine(currentAdmission);
    const rebuilt = buildLiveProviderIntentPublicationPlan(
      currentAdmission,
      fixtureOnly,
      operationPlan,
    );
    if (
      !sameLiveProviderIntentValue(currentAdmission, baselineAdmission) ||
      !sameLiveProviderIntentValue(rebuilt, publicationPlan)
    ) {
      fail("live provider intent observation changed before slot publication", 73);
    }
  };
  if (
    publicationPlan.admission.completion_projection.operation_plan_sha256 !==
    liveProviderPlanDigest(liveProviderPlanBytes(operationPlan))
  ) {
    fail("live provider intent completed plan binding changed", 73);
  }
  const expectedSource = Object.freeze({
    sequence: initial.receiptState.head.sequence,
    sha256: initial.receiptState.head_sha256,
  });
  let afterSlotLinkFailure;
  const held = acquireMutationLease(
    roots,
    COLIMA_LIVE_PROVIDER_INTENT_ACTION,
    ZERO_SHA256,
    expectedSource,
    {
      afterAuthorityObserver:
        testCheckpoint === undefined
          ? undefined
          : () => testCheckpoint("after-slot-authority-reassertion"),
      afterLinkObserver:
        testCheckpoint === undefined
          ? undefined
          : () => {
              try {
                testCheckpoint("after-slot-link");
              } catch (error) {
                afterSlotLinkFailure = error;
              }
            },
      beforeStageObserver:
        testCheckpoint === undefined
          ? undefined
          : () => testCheckpoint("before-slot-stage"),
      reassertAuthority: reassertSlotPublication,
    },
    Object.freeze({
      contractSha256: publicationPlan.operation_contract_sha256,
      kind: publicationPlan.operation_kind,
      plan: publicationPlan,
    }),
  );
  let publicationFailure;
  try {
    if (afterSlotLinkFailure !== undefined) throw afterSlotLinkFailure;
    const acquiredAdmission = observeLiveProviderIntentAdmission(
      admittedArguments,
      {
        boundary: "after-slot-acquisition",
        fixtureOnly,
        stateSnapshot: (state) =>
          heldLiveProviderIntentPlanSnapshot(
            state,
            held,
            publicationPlan,
            undefined,
            fixtureOnly,
          ),
        testCheckpoint,
      },
    );
    assertLiveProviderIntentAdmissionPristine(acquiredAdmission);
    const acquiredPlan = buildLiveProviderIntentPublicationPlan(
      acquiredAdmission,
      fixtureOnly,
      operationPlan,
    );
    if (
      !sameLiveProviderIntentValue(acquiredAdmission, baselineAdmission) ||
      !sameLiveProviderIntentValue(acquiredPlan, publicationPlan)
    ) {
      fail("live provider intent observation changed after slot acquisition", 73);
    }
    testCheckpoint?.("after-slot-acquisition");
    let afterCloseLinkFailure;
    closeMutationLease(
      roots,
      held,
      "completed",
      {
        afterLinkObserver:
          testCheckpoint === undefined
            ? undefined
            : () => {
                try {
                  testCheckpoint("after-close-link");
                } catch (error) {
                  afterCloseLinkFailure = error;
                }
              },
      },
      ZERO_SHA256,
      (state, ownStage) => {
        if (ownStage === undefined) return;
        const closeAdmission = observeLiveProviderIntentAdmission(
          admittedArguments,
          {
            boundary: "before-close-link",
            fixtureOnly,
            stateSnapshot: (current) =>
              heldLiveProviderIntentPlanSnapshot(
                current,
                held,
                publicationPlan,
                ownStage,
                fixtureOnly,
              ),
            testCheckpoint,
          },
        );
        assertLiveProviderIntentAdmissionPristine(closeAdmission);
        const closePlan = buildLiveProviderIntentPublicationPlan(
          closeAdmission,
          fixtureOnly,
          operationPlan,
        );
        if (
          !sameLiveProviderIntentValue(closeAdmission, baselineAdmission) ||
          !sameLiveProviderIntentValue(closeAdmission, acquiredAdmission) ||
          !sameLiveProviderIntentValue(closePlan, publicationPlan) ||
          state.mutationLease === undefined ||
          !state.mutationLease.bytes.equals(held.leaseBytes)
        ) {
          fail("live provider intent observation changed before close publication", 73);
        }
      },
    );
    if (afterCloseLinkFailure !== undefined) throw afterCloseLinkFailure;
  } catch (error) {
    publicationFailure = error;
  }
  if (publicationFailure !== undefined) {
    let state;
    try {
      state = loadState(roots, false);
    } catch (error) {
      throw error;
    }
    if (
      state.mutationLease !== undefined &&
      state.mutationLease.bytes.equals(held.leaseBytes)
    ) {
      closeMutationLease(
        roots,
        held,
        "aborted-before-effect",
      );
    }
    throw publicationFailure;
  }
  const verified = loadState(roots, true);
  const recorded = completedLiveProviderIntentForVariant(
    verified,
    fixtureOnly,
  );
  if (
    recorded === undefined ||
    !canonicalBytes(recorded.publicationPlan).equals(
      canonicalBytes(publicationPlan),
    )
  ) {
    fail("live provider intent publication was not durable", 70);
  }
  return liveProviderIntentCompletion(recorded);
}

export function publishColimaLiveProviderIntentForExecutor(argumentsValue) {
  return publishColimaLiveProviderIntent(
    admissionArguments(argumentsValue),
    { fixtureOnly: false, testCheckpoint: undefined },
  );
}

// This unsupported fixture seam replaces only the production-pinned root
// evidence. It has a distinct operation contract, persisted schema and result.
export function publishColimaLiveProviderIntentForTest(argumentsValue) {
  const admittedArguments = admissionArguments(argumentsValue, [
    "requirements",
    "testCheckpoint",
  ]);
  if (
    admittedArguments.requirements === null ||
    Array.isArray(admittedArguments.requirements) ||
    typeof admittedArguments.requirements !== "object" ||
    typeof admittedArguments.testCheckpoint !== "function"
  ) {
    fail("live provider fixture intent arguments were refused", 64);
  }
  return publishColimaLiveProviderIntent(
    admittedArguments,
    {
      fixtureOnly: true,
      testCheckpoint: admittedArguments.testCheckpoint,
    },
  );
}

export function executeProviderCreateForExecutor(argumentsValue) {
  validateFakeProviderAdapter(argumentsValue.adapter);
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const closePublicationHolds = {
    beforeLinkMilliseconds: argumentsValue.adapter.close_prelink_hold_milliseconds,
  };
  const initial = loadState(roots, true);
  refuseCompletedLiveProviderPlan(initial);
  if (
    initial.receiptState.head.phase !== "plan" ||
    initial.pendingPublication !== undefined ||
    initial.environment !== undefined ||
    initial.environmentPublication !== undefined ||
    initial.providerState.contract !== "synchronous-fake"
  ) {
    fail("provider creation state was refused", 73);
  }
  const intentResult = providerIntentResult(initial.candidate.run_id);
  let intendedReceipt;
  try {
    intendedReceipt = createNextReceipt(
      initial.receipts,
      initial.candidate.run_id,
      "provider-create-intent",
      intentResult,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  const held = acquireMutationLease(
    roots,
    "provider-create",
    digest(canonicalBytes(intendedReceipt)),
    {
      sequence: initial.receiptState.head.sequence,
      sha256: initial.receiptState.head_sha256,
    },
    {
      afterLinkMilliseconds: argumentsValue.adapter.publication_hold_milliseconds,
      beforeLinkMilliseconds: argumentsValue.adapter.prelink_hold_milliseconds,
    },
    {
      contractSha256: FAKE_PROVIDER_CONTRACT_SHA256,
      kind: DETERMINISTIC_PROVIDER_OPERATION_KIND,
      plan: null,
    },
  );
  if (
    held.lease.source_sequence !== initial.receiptState.head.sequence ||
    held.lease.source_head_sha256 !== initial.receiptState.head_sha256
  ) {
    closeMutationLease(roots, held, "aborted-before-effect", closePublicationHolds);
    fail("provider creation source head changed", 73);
  }
  // Provider exceptions deliberately keep the durable slot open. Only an
  // exact result receipt, or explicit identity-bound recovery, may close it.
  appendReceiptWithLease(roots, { phase: "provider-create-intent", result: intentResult });
  assertMutationLeaseHeld(roots, held);
  holdFakeProvider(argumentsValue.adapter.hold_milliseconds);
  assertMutationLeaseHeld(roots, held);
  const next = providerAdapterReceipt({
    outcome: argumentsValue.adapter.execute_outcome,
    result: argumentsValue.adapter.execute_result,
  });
  const receipt = appendReceiptWithLease(roots, next);
  assertMutationLeaseHeld(roots, held);
  closeMutationLease(roots, held, "completed", closePublicationHolds);
  return receipt;
}

function assertBackgroundOwnerAuthority(roots, held) {
  const state = assertMutationLeaseHeld(roots, held, false);
  if (
    state.mutationLease?.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
    state.mutationLease.value.operation_contract_sha256 !==
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256
  ) {
    fail("background provider owner authority was refused", 73);
  }
  return state;
}

function assertBackgroundOwnerExecutionAuthority(roots, held) {
  const state = assertBackgroundOwnerAuthority(roots, held);
  if (!sourceClosureMatches(roots, state.candidate)) {
    fail("background provider source closure changed", 73);
  }
  return state;
}

function backgroundProcessAuthorityGate(roots, held) {
  return (checkpoint) => {
    if (
      checkpoint === null ||
      Array.isArray(checkpoint) ||
      typeof checkpoint !== "object" ||
      !CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS.includes(checkpoint.checkpoint) ||
      !onlyLowerHex(checkpoint.evidence_head_sha256, 64)
    ) {
      fail("background provider process authority was refused", 70);
    }
    const state = assertBackgroundOwnerExecutionAuthority(roots, held);
    if (
      state.receiptState.head.phase !== "provider-create-intent" ||
      state.receiptState.head_sha256 !== held.lease.intent_receipt_sha256 ||
      state.operationSettlement !== undefined ||
      checkpoint.evidence_head_sha256 !== state.providerState.prefix.evidenceHeadSha256 ||
      (checkpoint.checkpoint === "before-create-authority-publication" &&
        checkpoint.authority_sha256 !==
          state.providerState.prefix.pendingPublication?.actual_sha256)
    ) {
      fail("background provider process authority changed", 73);
    }
  };
}

function finishBackgroundProviderMutation({
  adapter,
  assertAuthority,
  closeOperation,
  decision,
  roots,
}) {
  discardSupersededBackgroundReceiptPublication(roots, assertAuthority);
  let state = assertAuthority();
  let publishedSettlement = false;
  if (state.operationSettlement === undefined) {
    if (decision === undefined) {
      fail("background provider effect remained uncertain", 73);
    }
    state = publishBackgroundOperationSettlement(
      roots,
      state,
      state.mutationLease,
      {
        disposition: decision.disposition,
        reassertAuthority: assertAuthority,
        safeCode: decision.safeCode,
      },
    );
    publishedSettlement = true;
  }
  if (publishedSettlement) holdFakeProvider(adapter.after_settlement_hold_milliseconds);
  state = assertAuthority();
  const settlement = state.operationSettlement;
  const settlementSha256 = digest(settlement.bytes);
  const reassertReceipt = backgroundReceiptReassertion(roots, assertAuthority);
  let appended = false;
  if (state.receiptState.head.phase === "plan") {
    if (
      settlement.value.disposition !== "exact-residual" ||
      settlement.value.safe_code !== "resource-collision"
    ) {
      fail("background provider preflight settlement was refused", 73);
    }
    appendReceiptWithLease(
      roots,
      {
        phase: "preflight-refused",
        result: backgroundProviderFailureResult(settlement, "preflight-refused"),
      },
      { reassertAuthority: reassertReceipt },
    );
    appended = true;
  } else if (state.receiptState.head.phase === "provider-create-intent") {
    if (settlement.value.disposition === "complete-identity") {
      if (sourceClosureMatches(roots, state.candidate)) {
        try {
          appendReceiptWithLease(
            roots,
            {
              phase: "provider-create-passed",
              result: backgroundProviderPassedResult(state.mutationLease, settlement),
            },
            { checkSource: false, reassertAuthority: reassertReceipt },
          );
        } catch (error) {
          if (
            !(error instanceof ClosedFailure) ||
            error.message !== "background provider source closure changed"
          ) {
            throw error;
          }
          discardSupersededBackgroundReceiptPublication(roots, assertAuthority);
        }
      }
      state = assertAuthority();
      if (state.receiptState.head.phase === "provider-create-intent") {
        appendReceiptWithLease(
          roots,
          {
            phase: "execution-failed",
            result: backgroundProviderFailureResult(settlement, "execution-failed"),
          },
          { reassertAuthority: reassertReceipt },
        );
      }
    } else {
      appendReceiptWithLease(
        roots,
        {
          phase: "provider-create-failed",
          result: backgroundProviderFailureResult(settlement),
        },
        { reassertAuthority: reassertReceipt },
      );
    }
    appended = true;
  }
  if (appended) holdFakeProvider(adapter.after_result_hold_milliseconds);
  state = assertAuthority();
  if (
    state.receiptState.head.phase === "provider-create-passed" &&
    !sourceClosureMatches(roots, state.candidate)
  ) {
    appendReceiptWithLease(
      roots,
      {
        phase: "execution-failed",
        result: backgroundProviderFailureResult(settlement, "execution-failed"),
      },
      { reassertAuthority: reassertReceipt },
    );
    state = assertAuthority();
  }
  if (
    !new Set([
      "execution-failed",
      "preflight-refused",
      "provider-create-failed",
      "provider-create-passed",
    ]).has(state.receiptState.head.phase)
  ) {
    fail("background provider terminal receipt was refused", 73);
  }
  closeOperation(settlementSha256);
  return loadState(roots, false).receiptState.head;
}

export async function executeBackgroundProviderCreateForExecutor(argumentsValue) {
  validateBackgroundProviderAdapter(argumentsValue.adapter);
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const initial = loadState(roots, true);
  refuseCompletedLiveProviderPlan(initial);
  if (
    initial.receiptState.head.phase !== "plan" ||
    initial.pendingPublication !== undefined ||
    initial.environment !== undefined ||
    initial.environmentPublication !== undefined ||
    initial.providerState.contract !== "synchronous-fake"
  ) {
    fail("background provider creation state was refused", 73);
  }
  let operationPlan;
  try {
    operationPlan = planControlledBackgroundProviderOperation({
      evidenceDirectory: join(initial.run, "provider"),
      fixtureId: initial.candidate.run_id,
      ownershipNonce: randomBytes(32).toString("hex"),
      providerBase: argumentsValue.providerBase,
    });
  } catch (error) {
    if (error instanceof ProviderProcessContractFailure) fail(error.message, error.exitStatus);
    throw error;
  }
  const intentResult = {
    operation_kind: CONTROLLED_BACKGROUND_OPERATION_KIND,
    operation_plan_sha256: operationPlanSha256(operationPlan),
    preexisting_resource: "absent",
    provider_contract_sha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    provider_resource: operationPlan.provider_resource,
    provider_root_key: operationPlan.provider_root_key,
  };
  let intendedReceipt;
  try {
    intendedReceipt = createNextReceipt(
      initial.receipts,
      initial.candidate.run_id,
      "provider-create-intent",
      intentResult,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  const held = acquireMutationLease(
    roots,
    "provider-create",
    digest(canonicalBytes(intendedReceipt)),
    {
      sequence: initial.receiptState.head.sequence,
      sha256: initial.receiptState.head_sha256,
    },
    {},
    {
      contractSha256: CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
      kind: CONTROLLED_BACKGROUND_OPERATION_KIND,
      plan: operationPlan,
    },
  );
  const assertAuthority = () => assertBackgroundOwnerAuthority(roots, held);
  const assertExecutionAuthority = () =>
    assertBackgroundOwnerExecutionAuthority(roots, held);
  appendReceiptWithLease(
    roots,
    { phase: "provider-create-intent", result: intentResult },
    {
      reassertAuthority: backgroundReceiptReassertion(
        roots,
        assertExecutionAuthority,
      ),
    },
  );
  holdFakeProvider(argumentsValue.adapter.after_intent_hold_milliseconds);
  let state = assertAuthority();
  let decision = preIntentBackgroundResourceCollision(state.providerState.prefix)
    ? backgroundSettlementDecision(state.providerState.prefix)
    : undefined;
  if (decision === undefined) {
    try {
      planControlledBackgroundProviderCreateWithAuthorityGate(
        {
          bindings: state.providerState.bindings,
          evidenceDirectory: operationPlan.evidence_directory.path,
          fixtureId: operationPlan.fixture_id,
          operationPlan,
          providerBase: operationPlan.provider_base.path,
        },
        backgroundProcessAuthorityGate(roots, held),
      );
      holdFakeProvider(argumentsValue.adapter.after_authority_hold_milliseconds);
      await launchControlledBackgroundProviderWithAuthorityGate(
        {
          beforeDetachHoldMilliseconds:
            argumentsValue.adapter.before_detach_hold_milliseconds,
          beforeIdentityProbeHoldMilliseconds:
            argumentsValue.adapter.before_identity_probe_hold_milliseconds,
          beforeStartDecisionHoldMilliseconds:
            argumentsValue.adapter.before_start_decision_hold_milliseconds,
          beforeStartHoldMilliseconds: argumentsValue.adapter.before_start_hold_milliseconds,
          evidenceDirectory: operationPlan.evidence_directory.path,
          fixtureId: operationPlan.fixture_id,
          maximumLifetimeMilliseconds: argumentsValue.adapter.maximum_lifetime_milliseconds,
          providerBase: operationPlan.provider_base.path,
        },
        backgroundProcessAuthorityGate(roots, held),
      );
    } catch (error) {
      if (
        !(error instanceof ProviderProcessContractFailure) &&
        !(error instanceof ClosedFailure)
      ) {
        throw error;
      }
      state = assertAuthority();
      decision = backgroundSettlementDecision(state.providerState.prefix);
      if (decision === undefined) throw error;
    }
    state = assertAuthority();
    if (decision === undefined) decision = backgroundSettlementDecision(state.providerState.prefix);
  }
  holdFakeProvider(argumentsValue.adapter.after_evidence_hold_milliseconds);
  return finishBackgroundProviderMutation({
    adapter: argumentsValue.adapter,
    assertAuthority,
    closeOperation: (settlementSha256) =>
      closeMutationLease(roots, held, "completed", {}, settlementSha256),
    decision,
    roots,
  });
}

function exactActiveStateInventory(roots, state) {
  return canonical(readdirSync(roots.stateBase).sort()) ===
    canonical([`.run-${state.candidate.run_id}`, "active"].sort());
}

function assertBackgroundCleanupComplete(roots, state) {
  const cleanup = state.cleanupState;
  const prefix = cleanup.prefix;
  if (
    cleanup.contract !== "controlled-background-retirement" ||
    prefix.cleanupStage !== "settled" ||
    prefix.rootDisposition !== "retired" ||
    prefix.pendingProgress !== undefined ||
    prefix.providerSettlementPublication?.disposition !== "final"
  ) {
    fail("background cleanup lifecycle evidence was refused", 73);
  }
  if (
    prefix.providerSettlement === undefined ||
    prefix.providerSettlement.value.result_receipt_authorized !== false
  ) {
    fail("background cleanup inner settlement was refused", 73);
  }
  if (
    prefix.processResidual.controller_presence !==
      "retired-before-plan-publication" ||
    prefix.processResidual.hostagent_presence !==
      "retired-by-authorized-progress"
  ) {
    fail("background cleanup process residual was refused", 73);
  }
  if (
    prefix.createEvidenceHeadSha256 !==
      cleanup.operationPlan.provider_identity_sha256 ||
    prefix.providerSettlementSha256 === ZERO_SHA256
  ) {
    fail("background cleanup identity evidence was refused", 73);
  }
  if (!exactActiveStateInventory(roots, state)) {
    fail("background cleanup inert staging was refused", 73);
  }
  if (!sourceClosureMatches(roots, state.candidate)) {
    fail("background cleanup source closure changed", 73);
  }
  return state;
}

function validateBackgroundCleanupProspectiveReceipt(roots, state, receipt) {
  const slot = state.mutationLease;
  if (
    slot?.value.action !== "provider-cleanup" ||
    slot.value.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
    receipt.sequence !== state.receiptState.head.sequence + 1 ||
    receipt.previous_sha256 !== state.receiptState.head_sha256
  ) {
    fail("background cleanup receipt authority was refused", 73);
  }
  let expectedResult;
  if (receipt.phase === "provider-cleanup-intent") {
    if (
      state.receiptState.head.sequence !== slot.value.source_sequence ||
      state.receiptState.head_sha256 !== slot.value.source_head_sha256 ||
      state.receiptState.head.phase !== "project-cleanup-passed" ||
      state.cleanupState.settlement !== undefined ||
      state.cleanupState.prefix.cleanupStage !== "not-started"
    ) {
      fail("background cleanup intent authority was refused", 73);
    }
    expectedResult = backgroundCleanupIntentResult(slot);
  } else if (receipt.phase === "provider-cleanup-passed") {
    if (
      state.receiptState.head.phase !== "provider-cleanup-intent" ||
      state.receiptState.head_sha256 !== slot.value.intent_receipt_sha256 ||
      state.cleanupState.settlement === undefined
    ) {
      fail("background cleanup pass authority was refused", 73);
    }
    assertBackgroundCleanupComplete(roots, state);
    expectedResult = backgroundCleanupPassedResult(
      slot,
      state.cleanupState.settlement,
    );
  } else {
    fail("background cleanup receipt phase was refused", 73);
  }
  if (canonical(receipt.result) !== canonical(expectedResult)) {
    fail("background cleanup receipt result was refused", 73);
  }
}

function backgroundCleanupReceiptReassertion(roots, assertAuthority) {
  if (typeof assertAuthority !== "function") {
    fail("background cleanup receipt authority was refused", 70);
  }
  return (receipt) => {
    const state = assertAuthority();
    validateBackgroundCleanupProspectiveReceipt(roots, state, receipt);
    if (!sourceClosureMatches(roots, state.candidate)) {
      fail("background cleanup source closure changed", 73);
    }
  };
}

function assertBackgroundCleanupStateAuthority(
  roots,
  state,
  { requireExecution = false, requireSource = true } = {},
) {
  const slot = state.mutationLease;
  const operationPlan = slot?.value.operation_plan;
  if (
    slot?.value.action !== "provider-cleanup" ||
    slot.value.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
    slot.value.operation_contract_sha256 !==
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 ||
    state.cleanupState.contract !== "controlled-background-retirement" ||
    canonical(operationPlan) !== canonical(state.cleanupState.operationPlan) ||
    canonical(
      currentBoundDirectoryIdentity(
        operationPlan.evidence_directory.path,
        "background cleanup evidence directory",
      ),
    ) !== canonical(operationPlan.evidence_directory) ||
    canonical(
      currentBoundDirectoryIdentity(
        operationPlan.provider_base.path,
        "background cleanup provider base",
      ),
    ) !== canonical(operationPlan.provider_base) ||
    (requireSource && !sourceClosureMatches(roots, state.candidate)) ||
    (requireExecution &&
      (state.receiptState.head.phase !== "provider-cleanup-intent" ||
        state.receiptState.head_sha256 !==
          slot.value.intent_receipt_sha256 ||
        state.cleanupState.settlement !== undefined))
  ) {
    fail("background cleanup state authority was refused", 73);
  }
  return state;
}

function assertBackgroundCleanupOwnerAuthority(roots, held) {
  return assertBackgroundCleanupStateAuthority(
    roots,
    assertMutationLeaseHeld(roots, held, false),
  );
}

function assertBackgroundCleanupExecutionAuthority(roots, held) {
  return assertBackgroundCleanupStateAuthority(
    roots,
    assertMutationLeaseHeld(roots, held, false),
    { requireExecution: true },
  );
}

function backgroundCleanupResourceIdentitySha256(plan, resources) {
  const byPath = new Map(
    plan.root_inventory.map((entry) => [entry.relative_path, entry]),
  );
  const identities = resources.map((resource) => {
    if (resource === ".") return plan.root;
    const identity = byPath.get(resource);
    if (identity === undefined) {
      fail("background cleanup checkpoint resource was refused", 73);
    }
    return identity;
  });
  return providerProcessDigest(providerProcessBytes(identities));
}

function backgroundCleanupPublicationPhase(disposition) {
  const phases = new Map([
    ["absent", "before-stage-write"],
    ["final", "before-final-consumption"],
    ["linked-complete", "before-stage-removal"],
    ["redundant-complete", "before-stage-removal"],
    ["redundant-partial", "before-partial-stage-removal"],
    ["staged-complete", "before-final-link"],
    ["staged-partial", "before-partial-stage-removal"],
  ]);
  const phase = phases.get(disposition);
  if (phase === undefined) {
    fail("background cleanup publication frontier was refused", 73);
  }
  return phase;
}

function backgroundCleanupPublicationCheckpointFields(
  observation,
  targetName,
  expectedSha256,
) {
  const phase = backgroundCleanupPublicationPhase(observation.disposition);
  const finalPresent = new Set([
    "final",
    "linked-complete",
    "redundant-complete",
    "redundant-partial",
  ]).has(observation.disposition);
  const stagePresent = !new Set(["absent", "final"]).has(
    observation.disposition,
  );
  const completeStage = new Set([
    "linked-complete",
    "redundant-complete",
    "staged-complete",
  ]).has(observation.disposition);
  if (
    (finalPresent
      ? observation.final_sha256 !== expectedSha256 ||
        observation.final_identity_sha256 === ZERO_SHA256
      : observation.final_sha256 !== ZERO_SHA256 ||
        observation.final_identity_sha256 !== ZERO_SHA256) ||
    (stagePresent
      ? observation.stage_declared_sha256 !== expectedSha256 ||
        observation.stage_identity_sha256 === ZERO_SHA256 ||
        observation.stage_actual_sha256 === ZERO_SHA256
      : observation.stage_declared_sha256 !== ZERO_SHA256 ||
        observation.stage_identity_sha256 !== ZERO_SHA256 ||
        observation.stage_actual_sha256 !== ZERO_SHA256) ||
    (completeStage && observation.stage_actual_sha256 !== expectedSha256)
  ) {
    fail("background cleanup publication identity was refused", 73);
  }
  return Object.freeze({
    publication_disposition: observation.disposition,
    publication_expected_sha256: expectedSha256,
    publication_phase: phase,
    publication_stage_declared_sha256:
      observation.stage_declared_sha256,
    publication_stage_identity_sha256:
      observation.stage_identity_sha256,
    publication_stage_sha256: observation.stage_actual_sha256,
    publication_target_name: targetName,
  });
}

function backgroundCleanupEffectCheckpointFields() {
  return Object.freeze({
    publication_disposition: "not-applicable",
    publication_expected_sha256: ZERO_SHA256,
    publication_phase: "not-applicable",
    publication_stage_declared_sha256: ZERO_SHA256,
    publication_stage_identity_sha256: ZERO_SHA256,
    publication_stage_sha256: ZERO_SHA256,
    publication_target_name: "not-applicable",
  });
}

function backgroundCleanupProgressValue(prefix, recoveredAbsence) {
  const planArtifact = prefix.cleanupPlan;
  const sequence = prefix.completedSteps;
  const step = planArtifact?.value.retirement_steps[sequence];
  if (step === undefined || typeof recoveredAbsence !== "boolean") {
    fail("background cleanup progress frontier was refused", 73);
  }
  return Object.freeze({
    action: step.action,
    fixture_id: planArtifact.value.fixture_id,
    plan_sha256: planArtifact.sha256,
    previous_sha256:
      sequence === 0 ? planArtifact.sha256 : prefix.finalProgressSha256,
    recovered_absence: recoveredAbsence,
    resource_identity_sha256: backgroundCleanupResourceIdentitySha256(
      planArtifact.value,
      step.resources,
    ),
    resources: Object.freeze([...step.resources]),
    schema: "synveda.clean-engine.provider-retirement-step.v2",
    sequence,
  });
}

function backgroundCleanupInnerSettlementValue(prefix) {
  const plan = prefix.cleanupPlan?.value;
  if (plan === undefined || prefix.finalProgressSha256 === ZERO_SHA256) {
    fail("background cleanup settlement frontier was refused", 73);
  }
  return Object.freeze({
    cleanup_operation_plan_sha256: plan.cleanup_operation_plan_sha256,
    cleanup_plan_sha256: prefix.cleanupPlanSha256,
    cleanup_slot_sha256: plan.cleanup_slot_sha256,
    create_close_sha256: plan.create_close_sha256,
    create_settlement_sha256: plan.create_settlement_sha256,
    final_progress_sha256: prefix.finalProgressSha256,
    fixture_id: plan.fixture_id,
    outcome: "passed",
    provider_identity_sha256: plan.provider_identity_sha256,
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
    retirement_contract_sha256:
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    root_disposition: "retired",
    safe_code: "none",
    schema:
      "synveda.clean-engine.controlled-background-provider-retirement-settlement.v2",
    source_closure: "state-authority-reasserted",
    state_integration: "mutation-journal-v2",
  });
}

function backgroundCleanupProcessAuthorityGate(roots, assertAuthority) {
  if (typeof assertAuthority !== "function") {
    fail("background cleanup process authority was refused", 70);
  }
  const authorizedEffects = new Map();
  return (checkpoint) => {
    if (
      checkpoint === null ||
      Array.isArray(checkpoint) ||
      typeof checkpoint !== "object" ||
      !CONTROLLED_BACKGROUND_RETIREMENT_AUTHORITY_CHECKPOINTS.includes(
        checkpoint.checkpoint,
      )
    ) {
      fail("background cleanup process authority was refused", 70);
    }
    const state = assertAuthority();
    const slot = state.mutationLease;
    const bindings = backgroundCleanupBindings(slot);
    const prefix = state.cleanupState.prefix;
    const planArtifact = prefix.cleanupPlan;
    const plan = planArtifact?.value;
    const base = {
      cleanup_intent_sha256: bindings.cleanup_intent_sha256,
      cleanup_operation_plan_sha256: bindings.cleanup_operation_plan_sha256,
      cleanup_plan_sha256: prefix.cleanupPlanSha256,
      cleanup_slot_sequence: bindings.cleanup_slot_sequence,
      cleanup_slot_sha256: bindings.cleanup_slot_sha256,
      create_close_sha256: bindings.create_close_sha256,
      create_settlement_sha256: bindings.create_settlement_sha256,
      create_slot_sha256: bindings.create_slot_sha256,
      operation_kind: slot.value.operation_kind,
      provider_identity_sha256:
        state.cleanupState.operationPlan.provider_identity_sha256,
      retirement_contract_sha256: slot.value.operation_contract_sha256,
      source_head_sha256: bindings.source_head_sha256,
      source_sequence: bindings.source_sequence,
    };
    const candidates = [];
    if (
      checkpoint.checkpoint === "before-retirement-plan-publication" &&
      prefix.completedSteps === 0 &&
      prefix.providerSettlementSha256 === ZERO_SHA256
    ) {
      const expectedSha256 = prefix.cleanupPlanSha256;
      candidates.push({
        checkpoint: "before-retirement-plan-publication",
        ...base,
        completed_steps: 0,
        next_action: "publish-retirement-plan",
        next_resources: ["provider-retirement-plan.json"],
        ...backgroundCleanupPublicationCheckpointFields(
          prefix.observation.plan_publication,
          "provider-retirement-plan.json",
          expectedSha256,
        ),
        resource_identity_sha256: expectedSha256,
      });
    }
    const nextStep = prefix.observation.next_step;
    if (
      checkpoint.checkpoint === "before-hostagent-shutdown-delivery" &&
      plan !== undefined &&
      nextStep?.sequence === 0 &&
      nextStep.action === "authenticated-hostagent-stop" &&
      nextStep.disposition === "all-present" &&
      prefix.processResidual.hostagent_presence === "observed-present"
    ) {
      candidates.push({
        checkpoint: "before-hostagent-shutdown-delivery",
        ...base,
        completed_steps: 0,
        next_action: nextStep.action,
        next_resources: [...nextStep.resources],
        ...backgroundCleanupEffectCheckpointFields(),
        resource_identity_sha256: nextStep.resource_identity_sha256,
      });
    }
    if (
      checkpoint.checkpoint === "before-stale-socket-unlink" &&
      plan !== undefined &&
      nextStep?.sequence === 0 &&
      nextStep.action === "authenticated-hostagent-stop" &&
      new Set(["all-present", "partial"]).has(nextStep.disposition) &&
      prefix.processResidual.hostagent_presence === "proved-absent"
    ) {
      const absent = new Set(prefix.observation.absent_resources);
      for (const resource of nextStep.resources.filter(
        (candidate) => !absent.has(candidate),
      )) {
        candidates.push({
          checkpoint: "before-stale-socket-unlink",
          ...base,
          completed_steps: 0,
          next_action: "unlink-stale-socket",
          next_resources: [resource],
          ...backgroundCleanupEffectCheckpointFields(),
          resource_identity_sha256: backgroundCleanupResourceIdentitySha256(
            plan,
            [resource],
          ),
        });
      }
    }
    const effectCheckpointByAction = new Map([
      ["rmdir", "before-resource-rmdir"],
      ["rmdir-root", "before-resource-rmdir"],
      ["unlink", "before-resource-unlink"],
      ["unlink-owner", "before-resource-unlink"],
    ]);
    if (
      plan !== undefined &&
      nextStep !== null &&
      nextStep !== undefined &&
      nextStep.sequence === prefix.completedSteps &&
      nextStep.disposition === "all-present" &&
      effectCheckpointByAction.get(nextStep.action) === checkpoint.checkpoint
    ) {
      if (
        nextStep.action === "rmdir-root" &&
        (prefix.rootDisposition !== "owned" ||
          canonical(prefix.observation.absent_resources) !==
            canonical(
              plan.root_inventory
                .map((entry) => entry.relative_path)
                .sort(),
            ))
      ) {
        fail("background cleanup root retirement frontier was refused", 73);
      }
      candidates.push({
        checkpoint: checkpoint.checkpoint,
        ...base,
        completed_steps: prefix.completedSteps,
        next_action: nextStep.action,
        next_resources: [...nextStep.resources],
        ...backgroundCleanupEffectCheckpointFields(),
        resource_identity_sha256: nextStep.resource_identity_sha256,
      });
    }
    if (
      checkpoint.checkpoint === "before-retirement-progress-publication" &&
      plan !== undefined
    ) {
      if (checkpoint.publication_phase === "before-final-consumption") {
        const sequence = prefix.completedSteps - 1;
        const step = plan.retirement_steps[sequence];
        if (
          sequence >= 0 &&
          step !== undefined &&
          prefix.finalProgressSha256 !== ZERO_SHA256
        ) {
          candidates.push({
            checkpoint: "before-retirement-progress-publication",
            ...base,
            completed_steps: sequence,
            next_action: "publish-retirement-progress",
            next_resources: [...step.resources],
            publication_disposition: "final",
            publication_expected_sha256: prefix.finalProgressSha256,
            publication_phase: "before-final-consumption",
            publication_stage_declared_sha256: ZERO_SHA256,
            publication_stage_identity_sha256: ZERO_SHA256,
            publication_stage_sha256: ZERO_SHA256,
            publication_target_name: `retirement-step-${String(sequence).padStart(2, "0")}.json`,
            resource_identity_sha256: backgroundCleanupResourceIdentitySha256(
              plan,
              step.resources,
            ),
          });
        }
      } else if (
        nextStep?.sequence === prefix.completedSteps &&
        nextStep.disposition === "all-absent"
      ) {
        const effect = authorizedEffects.get(prefix.completedSteps);
        const recoveredAbsence =
          prefix.pendingProgress?.recoveredAbsence ??
          (prefix.completedSteps === 0
            ? effect !== "authenticated-hostagent-stop"
            : effect === undefined);
        const progressValue = backgroundCleanupProgressValue(
          prefix,
          recoveredAbsence,
        );
        const expectedSha256 = providerProcessDigest(
          providerProcessBytes(progressValue),
        );
        candidates.push({
          checkpoint: "before-retirement-progress-publication",
          ...base,
          completed_steps: prefix.completedSteps,
          next_action: "publish-retirement-progress",
          next_resources: [...nextStep.resources],
          ...backgroundCleanupPublicationCheckpointFields(
            prefix.observation.progress_publication,
            `retirement-step-${String(prefix.completedSteps).padStart(2, "0")}.json`,
            expectedSha256,
          ),
          resource_identity_sha256: nextStep.resource_identity_sha256,
        });
      }
    }
    if (
      checkpoint.checkpoint === "before-retirement-settlement-publication" &&
      plan !== undefined &&
      nextStep === null &&
      prefix.completedSteps === plan.retirement_steps.length &&
      prefix.rootDisposition === "retired"
    ) {
      const expectedSha256 = providerProcessDigest(
        providerProcessBytes(backgroundCleanupInnerSettlementValue(prefix)),
      );
      candidates.push({
        checkpoint: "before-retirement-settlement-publication",
        ...base,
        completed_steps: prefix.completedSteps,
        next_action: "publish-retirement-settlement",
        next_resources: ["provider-retirement-settlement.json"],
        ...backgroundCleanupPublicationCheckpointFields(
          prefix.observation.settlement_publication,
          "provider-retirement-settlement.json",
          expectedSha256,
        ),
        resource_identity_sha256: expectedSha256,
      });
    }
    if (!candidates.some((candidate) => canonical(candidate) === canonical(checkpoint))) {
      fail("background cleanup process authority changed", 73);
    }
    if (
      new Set([
        "before-hostagent-shutdown-delivery",
        "before-stale-socket-unlink",
        "before-resource-unlink",
        "before-resource-rmdir",
      ]).has(checkpoint.checkpoint)
    ) {
      authorizedEffects.set(
        checkpoint.completed_steps,
        checkpoint.next_action,
      );
    }
  };
}

function backgroundCleanupAuthorityGateWithTestObserver(gate, observer) {
  if (observer === undefined) return gate;
  if (typeof observer !== "function") {
    fail("background cleanup authority test observer was refused", 64);
  }
  return (checkpoint) => {
    observer(checkpoint, gate);
    return gate(checkpoint);
  };
}

function publishBackgroundCleanupSettlement(
  roots,
  state,
  slot,
  reassertAuthority,
) {
  if (
    typeof reassertAuthority !== "function" ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(slot.bytes) ||
    slot.value.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
    state.cleanupState.settlement !== undefined ||
    state.receiptState.head.phase !== "provider-cleanup-intent" ||
    state.receiptState.head_sha256 !== slot.value.intent_receipt_sha256
  ) {
    fail("background cleanup settlement authority was refused", 73);
  }
  assertBackgroundCleanupComplete(roots, state);
  const recovery = state.mutationRecoveries.at(-1);
  const prefix = state.cleanupState.prefix;
  const settlement = {
    authority: recovery === undefined ? "owner" : "recovery",
    authority_sha256:
      recovery === undefined ? digest(slot.bytes) : digest(recovery.bytes),
    cleanup_plan_sha256: prefix.cleanupPlanSha256,
    disposition: "complete-retirement",
    effect_disposition: "complete",
    effect_name: "provider-cleanup",
    evidence_head_sha256: prefix.createEvidenceHeadSha256,
    evidence_prefix_sha256: prefix.observationSha256,
    evidence_stage: "settled",
    fixture_id: slot.value.fixture_id,
    operation_contract_sha256: slot.value.operation_contract_sha256,
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(slot.value.operation_plan),
    provider_retirement_settlement_sha256:
      prefix.providerSettlementSha256,
    residual_sha256: prefix.remainingInventorySha256,
    result_receipt_authorized: true,
    root_disposition: "retired",
    safe_code: "none",
    schema: BACKGROUND_CLEANUP_SETTLEMENT_SCHEMA,
    slot_sequence: slot.value.journal_sequence,
    slot_sha256: digest(slot.bytes),
    source_head_sha256: slot.value.intent_receipt_sha256,
  };
  validateBackgroundCleanupSettlement(
    settlement,
    slot,
    slot.value.fixture_id,
  );
  const bytes = canonicalBytes(settlement);
  const name = mutationOperationFileName(slot.value.journal_sequence);
  const expectedObservationSha256 = prefix.observationSha256;
  const expectedInnerSettlementSha256 = prefix.providerSettlementSha256;
  const reassertSettlementPublication = () => {
    const current = reassertAuthority();
    assertBackgroundCleanupComplete(roots, current);
    if (
      current.cleanupState.settlement !== undefined ||
      current.cleanupState.prefix.observationSha256 !==
        expectedObservationSha256 ||
      current.cleanupState.prefix.providerSettlementSha256 !==
        expectedInnerSettlementSha256
    ) {
      fail("background cleanup settlement observation changed", 73);
    }
  };
  if (!publishMutationBlocker(state.run, name, bytes, {
    reassertAuthority: reassertSettlementPublication,
  })) {
    const existing = parseCanonical(
      join(state.run, name),
      "background cleanup settlement",
    );
    if (!existing.bytes.equals(bytes)) {
      fail("background cleanup settlement publication conflicted", 73);
    }
  }
  const verified = loadState(roots, false);
  if (
    verified.cleanupState.settlement === undefined ||
    !verified.cleanupState.settlement.bytes.equals(bytes)
  ) {
    fail("background cleanup settlement publication was not durable", 70);
  }
  return verified;
}

function callBackgroundCleanupMutationPublicationCheckpoint(
  observer,
  checkpoint,
) {
  if (observer === undefined) return;
  if (
    typeof observer !== "function" ||
    !new Set([
      "after-slot-authority-reassertion",
      "before-mutation-stage-reconciliation",
      "before-mutation-stage-validation",
      "before-slot-stage",
    ]).has(checkpoint)
  ) {
    fail("background cleanup mutation publication test observer was refused", 64);
  }
  if (observer(checkpoint) !== undefined) {
    fail("background cleanup mutation publication test observer returned a value", 70);
  }
}

export async function executeBackgroundProviderCleanupForExecutor(argumentsValue) {
  validateBackgroundCleanupAdapter(argumentsValue.adapter);
  const mutationPublicationCheckpoint = (checkpoint) =>
    callBackgroundCleanupMutationPublicationCheckpoint(
      argumentsValue.testMutationPublicationCheckpoint,
      checkpoint,
    );
  if (
    argumentsValue.testMutationPublicationCheckpoint !== undefined &&
    typeof argumentsValue.testMutationPublicationCheckpoint !== "function"
  ) {
    fail("background cleanup mutation publication test observer was refused", 64);
  }
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const initial = loadState(roots, true);
  const retryableCleanup =
    initial.cleanupState.contract === "journal-only" ||
    (initial.cleanupState.contract === "controlled-background-retirement" &&
      initial.cleanupState.close?.value.disposition === "aborted-before-effect" &&
      initial.cleanupState.intentReceipt === undefined &&
      initial.cleanupState.settlement === undefined &&
      initial.cleanupState.prefix.cleanupStage === "not-started");
  if (
    initial.receiptState.head.phase !== "project-cleanup-passed" ||
    initial.pendingPublication !== undefined ||
    initial.environment !== undefined ||
    initial.environmentPublication !== undefined ||
    initial.providerState.contract !== "controlled-background-fake" ||
    initial.providerState.createClose?.value.disposition !== "completed" ||
    initial.providerState.settlement?.value.disposition !==
      "complete-identity" ||
    initial.providerState.passedReceipt?.phase !== "provider-create-passed" ||
    !retryableCleanup
  ) {
    fail("background cleanup state was refused", 73);
  }
  const operationPlan = backgroundCleanupOperationPlan({
    createClose: initial.providerState.createClose,
    createEvidence: initial.providerState.createEvidence,
    createSettlement: initial.providerState.settlement,
    createSlot: initial.providerState.createSlot,
  });
  const intentResult = Object.freeze({
    operation_kind: CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND,
    operation_plan_sha256: operationPlanSha256(operationPlan),
    provider_resource: operationPlan.provider_resource,
    retirement_contract_sha256:
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
    scope: "exact-receipt-owned-only",
  });
  let intendedReceipt;
  try {
    intendedReceipt = createNextReceipt(
      initial.receipts,
      initial.candidate.run_id,
      "provider-cleanup-intent",
      intentResult,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  const expectedSource = Object.freeze({
    sequence: initial.receiptState.head.sequence,
    sha256: initial.receiptState.head_sha256,
  });
  const reassertSlotPublication = () => {
    const current = loadState(roots, true);
    const currentRetryableCleanup =
      current.cleanupState.contract === "journal-only" ||
      (current.cleanupState.contract === "controlled-background-retirement" &&
        current.cleanupState.close?.value.disposition ===
          "aborted-before-effect" &&
        current.cleanupState.intentReceipt === undefined &&
        current.cleanupState.settlement === undefined &&
        current.cleanupState.prefix.cleanupStage === "not-started");
    const currentPlan =
      current.providerState.contract === "controlled-background-fake" &&
      currentRetryableCleanup
        ? backgroundCleanupOperationPlan({
            createClose: current.providerState.createClose,
            createEvidence: current.providerState.createEvidence,
            createSettlement: current.providerState.settlement,
            createSlot: current.providerState.createSlot,
          })
        : undefined;
    if (
      current.receiptState.head.sequence !== expectedSource.sequence ||
      current.receiptState.head_sha256 !== expectedSource.sha256 ||
      current.receiptState.head.phase !== "project-cleanup-passed" ||
      currentPlan === undefined ||
      canonical(currentPlan) !== canonical(operationPlan)
    ) {
      fail("background cleanup slot authority changed", 73);
    }
  };
  const held = acquireMutationLease(
    roots,
    "provider-cleanup",
    digest(canonicalBytes(intendedReceipt)),
    expectedSource,
    {
      afterAuthorityObserver:
        argumentsValue.testMutationPublicationCheckpoint === undefined
          ? undefined
          : () => mutationPublicationCheckpoint(
              "after-slot-authority-reassertion",
            ),
      beforeStageObserver:
        argumentsValue.testMutationPublicationCheckpoint === undefined
          ? undefined
          : () => mutationPublicationCheckpoint("before-slot-stage"),
      reassertAuthority: reassertSlotPublication,
    },
    {
      contractSha256: CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256,
      kind: CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND,
      plan: operationPlan,
    },
    {
      afterPublishedInventorySnapshot:
        argumentsValue.testMutationPublicationCheckpoint === undefined
          ? undefined
          : () => mutationPublicationCheckpoint(
              "before-mutation-stage-validation",
            ),
      beforeMutationStageReconciliation:
        argumentsValue.testMutationPublicationCheckpoint === undefined
          ? undefined
          : () => mutationPublicationCheckpoint(
              "before-mutation-stage-reconciliation",
            ),
    },
  );
  holdFakeProvider(argumentsValue.adapter.after_slot_hold_milliseconds);
  const assertAuthority = () =>
    assertBackgroundCleanupOwnerAuthority(roots, held);
  const assertExecutionAuthority = () =>
    assertBackgroundCleanupExecutionAuthority(roots, held);
  appendReceiptWithLease(
    roots,
    { phase: "provider-cleanup-intent", result: intentResult },
    {
      reassertAuthority: backgroundCleanupReceiptReassertion(
        roots,
        assertAuthority,
      ),
    },
  );
  holdFakeProvider(argumentsValue.adapter.after_intent_hold_milliseconds);
  let state = assertExecutionAuthority();
  const gate = backgroundCleanupAuthorityGateWithTestObserver(
    backgroundCleanupProcessAuthorityGate(
      roots,
      assertExecutionAuthority,
    ),
    argumentsValue.testAuthorityCheckpointObserver,
  );
  if (
    new Set(["not-started", "plan-publication-pending"]).has(
      state.cleanupState.prefix.cleanupStage,
    )
  ) {
    await planControlledBackgroundRetirementWithAuthorityGate(
      {
        bindings: state.cleanupState.bindings,
        evidenceDirectory: operationPlan.evidence_directory.path,
        fixtureId: operationPlan.fixture_id,
        providerBase: operationPlan.provider_base.path,
      },
      gate,
    );
  }
  holdFakeProvider(argumentsValue.adapter.after_plan_hold_milliseconds);
  state = assertExecutionAuthority();
  const retired = await retireControlledBackgroundProviderWithAuthorityGate(
    {
      crashAfterDeleteSequence:
        argumentsValue.adapter.crash_after_delete_sequence ?? undefined,
      crashAfterDeleteSyscallSequence:
        argumentsValue.adapter.crash_after_delete_syscall_sequence ?? undefined,
      crashAfterHostagentSettlement:
        argumentsValue.adapter.crash_after_hostagent_settlement,
      evidenceDirectory: operationPlan.evidence_directory.path,
      fixtureId: operationPlan.fixture_id,
      providerBase: operationPlan.provider_base.path,
      stopAfterSequence:
        argumentsValue.adapter.stop_after_sequence ?? undefined,
    },
    gate,
  );
  if (retired.complete !== true) {
    fail("background cleanup retirement remained incomplete", 75);
  }
  holdFakeProvider(argumentsValue.adapter.after_retirement_hold_milliseconds);
  state = assertExecutionAuthority();
  assertBackgroundCleanupComplete(roots, state);
  state = publishBackgroundCleanupSettlement(
    roots,
    state,
    state.mutationLease,
    assertExecutionAuthority,
  );
  holdFakeProvider(argumentsValue.adapter.after_settlement_hold_milliseconds);
  const reassertReceipt = backgroundCleanupReceiptReassertion(
    roots,
    assertBackgroundCleanupOwnerAuthority.bind(undefined, roots, held),
  );
  const receipt = appendReceiptWithLease(
    roots,
    {
      phase: "provider-cleanup-passed",
      result: backgroundCleanupPassedResult(
        state.mutationLease,
        state.cleanupState.settlement,
      ),
    },
    { reassertAuthority: reassertReceipt },
  );
  holdFakeProvider(argumentsValue.adapter.after_result_hold_milliseconds);
  holdFakeProvider(argumentsValue.adapter.before_close_hold_milliseconds);
  state = assertBackgroundCleanupOwnerAuthority(roots, held);
  assertBackgroundCleanupComplete(roots, state);
  closeMutationLease(
    roots,
    held,
    "completed",
    {
      beforeLinkMilliseconds:
        argumentsValue.adapter.close_prelink_hold_milliseconds,
    },
    state.cleanupState.operationEvidenceSha256,
  );
  return receipt;
}

function sourceClosureMatches(roots, candidate) {
  try {
    return canonical(sourceClosure(roots.repoRoot)) === canonical(candidate.source);
  } catch (error) {
    if (!(error instanceof ClosedFailure)) throw error;
    return false;
  }
}

function providerRecoveryBase(state) {
  const lease = state.mutationLease;
  if (lease === undefined) {
    fail("no abandoned provider mutation was available", 73);
  }
  const recoverableCreate = lease.value.action === "provider-create";
  const recoverablePlan = lease.value.action === LIVE_PROVIDER_PLAN_ACTION;
  const recoverableIntent =
    lease.value.action === COLIMA_LIVE_PROVIDER_INTENT_ACTION;
  const recoverableStartDecision =
    lease.value.action === COLIMA_LIVE_PROVIDER_START_DECISION_ACTION;
  const recoverableReservation =
    lease.value.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION;
  const recoverableEffect =
    lease.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION &&
    liveProviderEffectFixtureOnly(
      lease.value.operation_kind,
      lease.value.operation_contract_sha256,
    ) === true;
  const recoverableCleanup =
    lease.value.action === "provider-cleanup" &&
    lease.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND;
  if (
    !recoverableCreate &&
    !recoverablePlan &&
    !recoverableIntent &&
    !recoverableStartDecision &&
    !recoverableReservation &&
    !recoverableEffect &&
    !recoverableCleanup
  ) {
    fail("mutation recovery action was refused", 73);
  }
  return {
    fixtureId: lease.value.fixture_id,
    lease,
    leaseSha256: digest(lease.bytes),
    operation: Object.freeze({
      action: lease.value.action,
      operation_contract_sha256: lease.value.operation_contract_sha256,
      operation_kind: lease.value.operation_kind,
      operation_plan: lease.value.operation_plan,
    }),
    slotSequence: lease.value.journal_sequence,
  };
}

function liveProviderReservationEvidenceStage({
  previousStage,
  settlement,
  stage,
  witness,
}) {
  let evidenceStage;
  if (stage === undefined && witness === undefined) {
    evidenceStage = new Set([
      "stage-only",
      "stage-retired-before-effect",
    ]).has(previousStage)
      ? "stage-retired-before-effect"
      : "not-started";
  } else if (stage !== undefined && witness === undefined) {
    evidenceStage =
      stage.metadata.nlink === 1n ? "stage-only" : "marker-linked";
  } else if (stage !== undefined && witness !== undefined) {
    evidenceStage = "witness-linked";
  } else if (settlement === undefined) {
    evidenceStage = witness.metadata.nlink === 2n
      ? new Set(["marker-reacquired", "witness-unlinked"]).has(
          previousStage,
        )
        ? "marker-reacquired"
        : "marker-held"
      : "witness-unlinked";
  } else {
    evidenceStage =
      witness.metadata.nlink === 2n
        ? "retirement-authorized-held"
        : "retirement-authorized-retired";
  }
  return evidenceStage;
}

function liveProviderReservationRecoveryObservation(state) {
  const reservation =
    state.liveProviderReservation ?? state.liveProviderFixtureReservation;
  if (
    reservation === undefined ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(reservation.slot.bytes)
  ) {
    fail("live provider reservation recovery state was refused", 73);
  }
  const stage = state.providerReservationStage;
  const witness = state.providerReservationWitness;
  const settlement = reservation.settlement;
  const previousStage =
    state.mutationRecoveries.at(-1)?.value.observed_evidence_stage;
  const evidenceStage = liveProviderReservationEvidenceStage({
    previousStage,
    settlement,
    stage,
    witness,
  });
  const evidence = witness ?? stage;
  const evidenceSha256 =
    evidence === undefined
      ? evidenceStage === "stage-retired-before-effect"
        ? state.mutationRecoveries.at(-1).value
            .observed_evidence_head_sha256
        : ZERO_SHA256
      : digest(evidence.bytes);
  const localLinks =
    evidence === undefined ? 0 : Number(evidence.metadata.nlink);
  const settlementSha256 =
    settlement === undefined ? ZERO_SHA256 : digest(settlement.bytes);
  const topology =
    evidenceStage === "not-started"
      ? undefined
      : {
          evidence_sha256: evidenceSha256,
          evidence_stage: evidenceStage,
          local_links: localLinks,
          settlement_sha256: settlementSha256,
          slot_sequence: reservation.slot.value.journal_sequence,
        };
  const topologySha256 =
    topology === undefined ? ZERO_SHA256 : digest(canonicalBytes(topology));
  return Object.freeze({
    observed_effect_disposition:
      new Set([
        "not-started",
        "stage-only",
        "stage-retired-before-effect",
      ]).has(evidenceStage)
        ? "not-reached"
        : evidenceStage === "retirement-authorized-retired"
          ? "complete"
          : "pending",
    observed_effect_name: "provider-reservation",
    observed_evidence_head_sha256: evidenceSha256,
    observed_evidence_prefix_sha256: topologySha256,
    observed_evidence_stage: evidenceStage,
    observed_residual_sha256: topologySha256,
    observed_settlement_sha256: settlementSha256,
  });
}

function liveProviderEffectRecoveryObservation(state) {
  const effect = state.liveProviderFixtureEffect;
  if (
    effect === undefined ||
    effect.close !== undefined ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(effect.slot.bytes)
  ) {
    fail("fixture provider effect recovery state was refused", 73);
  }
  const evidenceStage = effect.currentStage;
  const descriptor = PROVIDER_EFFECT_RECOVERY_STAGES[evidenceStage];
  if (descriptor === undefined) {
    fail("fixture provider effect recovery stage was refused", 73);
  }
  const previous = state.mutationRecoveries.at(-1);
  let witnessSha256;
  if (effect.witness !== undefined) {
    witnessSha256 = digest(effect.witness.bytes);
  } else if (effect.stage?.initializing === true) {
    witnessSha256 = providerEffectInitializingStageSha256(
      effect.stage,
      effect.slot,
    );
  } else if (effect.stage !== undefined) {
    witnessSha256 = digest(effect.stage.bytes);
  } else if (evidenceStage === "stage-retired-before-effect") {
    witnessSha256 = previous?.value.observed_evidence_head_sha256;
  } else {
    witnessSha256 = ZERO_SHA256;
  }
  if (!onlyLowerHex(witnessSha256, 64)) {
    fail("fixture provider effect recovery witness was refused", 73);
  }
  const eventHeadSha256 =
    effect.events.length === 0
      ? ZERO_SHA256
      : digest(effect.events.at(-1).bytes);
  const evidenceHeadSha256 =
    eventHeadSha256 === ZERO_SHA256 ? witnessSha256 : eventHeadSha256;
  const markerPresent = new Set([
    "attempt-fenced",
    "authority-held",
    "marker-held",
    "marker-linked",
    "witness-linked",
  ]).has(evidenceStage);
  const topologySha256 =
    evidenceStage === "not-started"
      ? ZERO_SHA256
      : effectRecoveryTopologySha256({
          eventCount: effect.events.length,
          eventHeadSha256,
          evidenceStage,
          localLinks: descriptor.links,
          markerPresent,
          slotSequence: effect.slot.value.journal_sequence,
          witnessSha256,
        });
  return Object.freeze({
    observed_effect_disposition: descriptor.disposition,
    observed_effect_name: "provider-effect",
    observed_evidence_head_sha256: evidenceHeadSha256,
    observed_evidence_prefix_sha256: topologySha256,
    observed_evidence_stage: evidenceStage,
    observed_residual_sha256: topologySha256,
    observed_settlement_sha256: ZERO_SHA256,
  });
}

function providerRecoveryObservation(state) {
  if (
    state.mutationLease?.value.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION
  ) {
    return liveProviderEffectRecoveryObservation(state);
  }
  if (
    state.mutationLease?.value.action ===
    COLIMA_LIVE_PROVIDER_RESERVATION_ACTION
  ) {
    return liveProviderReservationRecoveryObservation(state);
  }
  if (
    state.mutationLease?.value.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
  ) {
    return backgroundCleanupPrefixObservation(
      state.cleanupState.prefix,
      state.cleanupState.settlement,
    );
  }
  if (state.mutationLease?.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND) {
    return Object.freeze({
      observed_effect_disposition: "not-reached",
      observed_effect_name: "none",
      observed_evidence_head_sha256: ZERO_SHA256,
      observed_evidence_prefix_sha256: ZERO_SHA256,
      observed_evidence_stage: "none",
      observed_residual_sha256: ZERO_SHA256,
      observed_settlement_sha256: ZERO_SHA256,
    });
  }
  const settlement = state.operationSettlement;
  if (settlement !== undefined) {
    return settlementObservation(settlement);
  }
  return backgroundPrefixObservation(state.providerState.prefix);
}

function providerRecoveryConfirmation(base) {
  return `recover:${base.fixtureId}:${String(base.slotSequence).padStart(2, "0")}:${base.leaseSha256}`;
}

export function providerRecoveryConfirmationForExecutor(argumentsValue) {
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (base.operation.action === COLIMA_LIVE_PROVIDER_RESERVATION_ACTION) {
    fail("live provider reservation requires its physical recovery confirmation", 73);
  }
  if (base.operation.action === COLIMA_LIVE_PROVIDER_EFFECT_ACTION) {
    fail("fixture provider effect requires its physical recovery confirmation", 73);
  }
  return providerRecoveryConfirmation(base);
}

export function liveProviderPlanRecoveryConfirmationForExecutor(argumentsValue) {
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (base.operation.action !== LIVE_PROVIDER_PLAN_ACTION) {
    fail("live provider plan recovery action was refused", 73);
  }
  return providerRecoveryConfirmation(base);
}

export function liveProviderIntentRecoveryConfirmationForExecutor(
  argumentsValue,
) {
  exactKeys(
    argumentsValue,
    ["repoRoot", "stateBase"],
    "live provider intent recovery confirmation arguments",
  );
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (base.operation.action !== COLIMA_LIVE_PROVIDER_INTENT_ACTION) {
    fail("live provider intent recovery action was refused", 73);
  }
  return providerRecoveryConfirmation(base);
}

function liveProviderStartDecisionRecoveryConfirmation(
  argumentsValue,
  fixtureOnly,
) {
  exactKeys(
    argumentsValue,
    ["repoRoot", "stateBase"],
    "live provider start decision recovery confirmation arguments",
  );
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (
    base.operation.action !== COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
    liveProviderStartDecisionFixtureOnly(
      base.operation.operation_kind,
      base.operation.operation_contract_sha256,
    ) !== fixtureOnly
  ) {
    fail("live provider start decision recovery action was refused", 73);
  }
  return providerRecoveryConfirmation(base);
}

export function liveProviderStartDecisionRecoveryConfirmationForExecutor(
  argumentsValue,
) {
  return liveProviderStartDecisionRecoveryConfirmation(argumentsValue, false);
}

export function liveProviderFixtureStartDecisionRecoveryConfirmationForTest(
  argumentsValue,
) {
  return liveProviderStartDecisionRecoveryConfirmation(argumentsValue, true);
}

function validateLiveProviderReservationRecoveryArguments(
  argumentsValue,
  fixtureOnly,
  { confirmation, testCheckpoint },
) {
  const fields = [
    "observation",
    "observationInput",
    "repoRoot",
    "stateBase",
    ...(fixtureOnly ? ["requirements"] : []),
    ...(confirmation ? ["confirmation"] : []),
    ...(testCheckpoint ? ["testCheckpoint"] : []),
  ];
  exactKeys(
    argumentsValue,
    fields,
    "live provider reservation recovery arguments",
  );
  if (
    typeof argumentsValue.repoRoot !== "string" ||
    typeof argumentsValue.stateBase !== "string" ||
    argumentsValue.observation === null ||
    Array.isArray(argumentsValue.observation) ||
    typeof argumentsValue.observation !== "object" ||
    argumentsValue.observationInput === null ||
    Array.isArray(argumentsValue.observationInput) ||
    typeof argumentsValue.observationInput !== "object" ||
    (fixtureOnly &&
      (argumentsValue.requirements === null ||
        Array.isArray(argumentsValue.requirements) ||
        typeof argumentsValue.requirements !== "object")) ||
    (confirmation && typeof argumentsValue.confirmation !== "string") ||
    (testCheckpoint && typeof argumentsValue.testCheckpoint !== "function")
  ) {
    fail("live provider reservation recovery arguments were refused", 64);
  }
  return argumentsValue;
}

function validateLiveProviderReservationRecoveryPhysicalState(
  state,
  argumentsValue,
  fixtureOnly,
) {
  const reservation = completedLiveProviderReservationForVariant(
    state,
    fixtureOnly,
  );
  if (reservation === undefined) {
    fail("live provider reservation recovery state was unavailable", 73);
  }
  assertLiveProviderReservationOpenState(
    state,
    fixtureOnly,
    reservation.publicationPlan,
  );
  validateLiveProviderReservationArgumentBinding(
    argumentsValue,
    reservation.source,
    fixtureOnly,
  );
  const stage = state.providerReservationStage;
  const witness = state.providerReservationWitness;
  if (stage === undefined && witness === undefined) {
    assertLiveProviderReservationMarkerAbsent(
      state,
      reservation.publicationPlan,
      argumentsValue,
    );
  } else if (stage !== undefined && witness === undefined) {
    if (stage.metadata.nlink === 1n) {
      assertLiveProviderReservationMarkerAbsent(
        state,
        reservation.publicationPlan,
        argumentsValue,
      );
      const { runMetadata } = assertLiveProviderReservationBoundDirectories(
        state,
        reservation.publicationPlan,
        argumentsValue,
      );
      inspectLiveProviderReservationArtifact(
        stage.path,
        "live provider reservation stage",
        runMetadata.dev,
        1,
        stage.bytes,
        stage.metadata,
      );
    } else {
      assertLiveProviderReservationLinkedTopology(
        state,
        reservation.publicationPlan,
        argumentsValue,
        2,
      );
    }
  } else if (stage !== undefined && witness !== undefined) {
    assertLiveProviderReservationLinkedTopology(
      state,
      reservation.publicationPlan,
      argumentsValue,
      3,
    );
  } else if (witness.metadata.nlink === 2n) {
    assertLiveProviderReservationLinkedTopology(
      state,
      reservation.publicationPlan,
      argumentsValue,
      2,
    );
  } else {
    assertLiveProviderReservationRetiredWitness(
      state,
      reservation.publicationPlan,
      argumentsValue,
    );
    if (reservation.settlement === undefined) {
      assertLiveProviderReservationMarkerAbsent(
        state,
        reservation.publicationPlan,
        argumentsValue,
      );
    }
  }
  return Object.freeze({
    observation: liveProviderReservationRecoveryObservation(state),
    reservation,
  });
}

function liveProviderReservationRecoveryConfirmation(
  argumentsValue,
  fixtureOnly,
) {
  const admittedArguments =
    validateLiveProviderReservationRecoveryArguments(
      argumentsValue,
      fixtureOnly,
      { confirmation: false, testCheckpoint: false },
    );
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (
    base.operation.action !== COLIMA_LIVE_PROVIDER_RESERVATION_ACTION ||
    liveProviderReservationFixtureOnly(
      base.operation.operation_kind,
      base.operation.operation_contract_sha256,
    ) !== fixtureOnly
  ) {
    fail("live provider reservation recovery action was refused", 73);
  }
  validateLiveProviderReservationRecoveryPhysicalState(
    state,
    admittedArguments,
    fixtureOnly,
  );
  return providerRecoveryConfirmation(base);
}

export function liveProviderReservationRecoveryConfirmationForExecutor(
  argumentsValue,
) {
  return liveProviderReservationRecoveryConfirmation(argumentsValue, false);
}

export function liveProviderFixtureReservationRecoveryConfirmationForTest(
  argumentsValue,
) {
  return liveProviderReservationRecoveryConfirmation(argumentsValue, true);
}

function validateLiveProviderEffectRecoveryArguments(
  argumentsValue,
  { confirmation, testCheckpoint },
) {
  const fields = [
    "observation",
    "observationInput",
    "repoRoot",
    "requirements",
    "stateBase",
    ...(confirmation ? ["confirmation"] : []),
    ...(testCheckpoint ? ["testCheckpoint"] : []),
  ];
  exactKeys(
    argumentsValue,
    fields,
    "fixture provider effect recovery arguments",
  );
  if (
    typeof argumentsValue.repoRoot !== "string" ||
    typeof argumentsValue.stateBase !== "string" ||
    argumentsValue.observation === null ||
    Array.isArray(argumentsValue.observation) ||
    typeof argumentsValue.observation !== "object" ||
    argumentsValue.observationInput === null ||
    Array.isArray(argumentsValue.observationInput) ||
    typeof argumentsValue.observationInput !== "object" ||
    argumentsValue.requirements === null ||
    Array.isArray(argumentsValue.requirements) ||
    typeof argumentsValue.requirements !== "object" ||
    (confirmation && typeof argumentsValue.confirmation !== "string") ||
    (testCheckpoint && typeof argumentsValue.testCheckpoint !== "function")
  ) {
    fail("fixture provider effect recovery arguments were refused", 64);
  }
  return argumentsValue;
}

function validateLiveProviderEffectRecoveryPhysicalState(
  state,
  argumentsValue,
  { permitAttemptFence = false } = {},
) {
  const effect = state.liveProviderFixtureEffect;
  if (effect === undefined) {
    fail("fixture provider effect recovery state was unavailable", 73);
  }
  assertLiveProviderEffectOpenState(state, effect.publicationPlan);
  validateLiveProviderReservationArgumentBinding(
    argumentsValue,
    effect.source,
    true,
  );
  const stage = state.providerEffectStage;
  const witness = state.providerEffectWitness;
  let markerDisposition;
  if (stage === undefined && witness === undefined) {
    markerDisposition = "absent";
    assertLiveProviderEffectMarkerAbsent(
      state,
      effect.publicationPlan,
      argumentsValue,
    );
  } else if (stage !== undefined && witness === undefined) {
    if (stage.metadata.nlink === 1n) {
      markerDisposition = "absent";
      const { runMetadata } = assertLiveProviderEffectBoundDirectories(
        state,
        effect.publicationPlan,
        argumentsValue,
      );
      const current = inspectPendingFile(
        stage.path,
        "fixture provider effect stage",
        runMetadata.dev,
        new Set([1n]),
      );
      if (
        !sameMutationArtifact(current, stage.metadata) ||
        !readPrivate(
          stage.path,
          "fixture provider effect stage",
          1,
          0n,
        ).equals(stage.bytes)
      ) {
        fail("fixture provider effect stage changed", 73);
      }
      assertLiveProviderEffectMarkerAbsent(
        state,
        effect.publicationPlan,
        argumentsValue,
      );
    } else {
      markerDisposition = "present";
      assertLiveProviderEffectLinkedTopology(
        state,
        effect.publicationPlan,
        argumentsValue,
        2,
      );
    }
  } else if (stage !== undefined && witness !== undefined) {
    markerDisposition = "present";
    assertLiveProviderEffectLinkedTopology(
      state,
      effect.publicationPlan,
      argumentsValue,
      3,
    );
  } else if (witness.metadata.nlink === 2n) {
    markerDisposition = "present";
    assertLiveProviderEffectLinkedTopology(
      state,
      effect.publicationPlan,
      argumentsValue,
      2,
    );
  } else {
    markerDisposition = "absent";
    assertLiveProviderEffectRetiredWitness(
      state,
      effect.publicationPlan,
      argumentsValue,
    );
  }
  observeLiveProviderEffectPhysicalRoots(
    argumentsValue,
    state,
    effect.source,
    effect.publicationPlan,
    markerDisposition,
  );
  const observation = liveProviderEffectRecoveryObservation(state);
  if (
    observation.observed_evidence_stage === "attempt-fenced" &&
    !permitAttemptFence
  ) {
    fail("fixture provider effect attempt fence requires operator resolution", 73);
  }
  return Object.freeze({ effect, observation });
}

export function liveProviderFixtureEffectRecoveryConfirmationForTest(
  argumentsValue,
) {
  const admittedArguments = validateLiveProviderEffectRecoveryArguments(
    argumentsValue,
    { confirmation: false, testCheckpoint: false },
  );
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (
    base.operation.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
    liveProviderEffectFixtureOnly(
      base.operation.operation_kind,
      base.operation.operation_contract_sha256,
    ) !== true
  ) {
    fail("fixture provider effect recovery action was refused", 73);
  }
  validateLiveProviderEffectRecoveryPhysicalState(
    state,
    admittedArguments,
  );
  return providerRecoveryConfirmation(base);
}

export function backgroundProviderCleanupRecoveryConfirmationForExecutor(
  argumentsValue,
) {
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const state = loadState(roots, false);
  const base = providerRecoveryBase(state);
  if (
    base.operation.action !== "provider-cleanup" ||
    base.operation.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND
  ) {
    fail("background cleanup recovery action was refused", 73);
  }
  return providerRecoveryConfirmation(base);
}

function acquireProviderRecovery(
  roots,
  confirmation,
  ownerProbe,
  publicationHolds = {},
  additionalReassertObservation = undefined,
  requireLeaseAndLatestAbsent = false,
) {
  if (
    additionalReassertObservation !== undefined &&
    typeof additionalReassertObservation !== "function"
  ) {
    fail("provider recovery physical observer was refused", 70);
  }
  if (typeof requireLeaseAndLatestAbsent !== "boolean") {
    fail("provider recovery predecessor policy was refused", 70);
  }
  const assertAbsentPredecessors = (base, latest) => {
    const owners = requireLeaseAndLatestAbsent
      ? [base.lease.value, latest?.value].filter((owner) => owner !== undefined)
      : [latest?.value ?? base.lease.value];
    for (const owner of owners) {
      const ownerState = mutationOwnerState(owner, ownerProbe);
      if (ownerState === "current" || ownerState === "unknown") {
        fail(
          requireLeaseAndLatestAbsent
            ? "provider recovery predecessor is active or could not be identified"
            : latest === undefined
              ? "provider mutation owner is active or could not be identified"
              : "another provider recovery is active or could not be identified",
          73,
        );
      }
    }
  };
  let state = loadState(roots, false);
  const observedBase = providerRecoveryBase(state);
  if (confirmation !== providerRecoveryConfirmation(observedBase)) {
    fail("provider recovery confirmation was refused", 64);
  }
  const observedLatest = state.mutationRecoveries.at(-1);
  assertAbsentPredecessors(observedBase, observedLatest);
  state = reconcileMutationStages(roots);
  const base = providerRecoveryBase(state);
  const latest = state.mutationRecoveries.at(-1);
  if (
    !base.lease.bytes.equals(observedBase.lease.bytes) ||
    (observedLatest === undefined
      ? latest !== undefined
      : latest === undefined || !latest.bytes.equals(observedLatest.bytes))
  ) {
    fail("provider recovery predecessor changed during reconciliation", 73);
  }
  assertAbsentPredecessors(base, latest);
  const observation = providerRecoveryObservation(state);
  const sequence = (latest?.value.sequence ?? -1) + 1;
  const cleanupNeedsFinalObservationClaim =
    base.operation.operation_kind ===
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND &&
    state.cleanupState.settlement === undefined &&
    state.cleanupState.prefix.cleanupStage !== "settled" &&
    !(
      state.receiptState.head.sequence === base.lease.value.source_sequence &&
      state.receiptState.head_sha256 === base.lease.value.source_head_sha256 &&
      state.pendingPublication === undefined &&
      state.cleanupState.prefix.cleanupStage === "not-started"
    );
  if (
    sequence >= MAX_MUTATION_RECOVERIES ||
    (cleanupNeedsFinalObservationClaim &&
      sequence >= MAX_MUTATION_RECOVERIES - 1)
  ) {
    fail("provider mutation recovery capacity was exhausted", 73);
  }
  const owner = currentProcessIdentity();
  const claim = {
    action: base.operation.action,
    chain_root_sha256: recoveryChainRootSha256(
      base.fixtureId,
      base.leaseSha256,
      base.operation,
    ),
    fixture_id: base.fixtureId,
    lease_sha256: base.leaseSha256,
    nonce: randomBytes(16).toString("hex"),
    ...observation,
    operation_contract_sha256: base.operation.operation_contract_sha256,
    operation_kind: base.operation.operation_kind,
    operation_plan_sha256: operationPlanSha256(base.operation.operation_plan),
    owner_boot_sha256: owner.boot_sha256,
    owner_instance_sha256: owner.instance_sha256,
    owner_pid: owner.pid,
    owner_probe: owner.probe,
    parent_sha256: latest === undefined ? ZERO_SHA256 : digest(latest.bytes),
    schema: MUTATION_RECOVERY_SCHEMA,
    sequence,
    slot_sequence: base.slotSequence,
    source_head_sha256: state.receiptState.head_sha256,
  };
  const bytes = canonicalBytes(claim);
  const name = recoveryFileName(base.slotSequence, sequence);
  const path = join(state.run, name);
  const reassertClaimPublication = () => {
    const current = loadState(roots, false);
    const currentLatest = current.mutationRecoveries.at(-1);
    if (
      current.mutationLease === undefined ||
      !current.mutationLease.bytes.equals(base.lease.bytes) ||
      current.receiptState.head_sha256 !== claim.source_head_sha256 ||
      (latest === undefined
        ? currentLatest !== undefined
        : currentLatest === undefined || !currentLatest.bytes.equals(latest.bytes)) ||
      canonical(providerRecoveryObservation(current)) !== canonical(observation)
    ) {
      fail("provider recovery observation changed before claim publication", 73);
    }
    assertAbsentPredecessors(base, latest);
    if (additionalReassertObservation?.(current) !== undefined) {
      fail("provider recovery physical observer returned a value", 70);
    }
  };
  if (!publishMutationBlocker(state.run, name, bytes, {
    ...publicationHolds,
    reassertAuthority: reassertClaimPublication,
  })) {
    fail("another provider recovery won the mutation claim", 73);
  }
  const identity = mutationHeldIdentity(path);
  state = loadState(roots, false);
  const durableClaim = state.allMutationRecoveries.find(
    (candidate) => candidate.name === name && candidate.bytes.equals(bytes),
  );
  if (durableClaim === undefined) {
    fail("provider recovery claim publication was not durable", 70);
  }
  if (state.mutationLease === undefined) {
    fail("provider mutation closed before the recovery claim became authoritative", 73);
  }
  const published = state.mutationRecoveries.at(-1);
  if (published === undefined || !published.bytes.equals(bytes)) {
    fail("provider recovery claim ownership was refused", 73);
  }
  const held = Object.freeze({
    base,
    bytes,
    claim,
    identity,
    observation,
    path,
    predecessor: latest,
  });
  const predecessorPresent =
    state.mutationLease?.bytes.equals(base.lease.bytes) === true &&
    (latest === undefined ||
      state.mutationRecoveries.some((recovery) => recovery.bytes.equals(latest.bytes)));
  if (
    !predecessorPresent ||
    state.receiptState.head_sha256 !== claim.source_head_sha256 ||
    canonical(providerRecoveryObservation(state)) !== canonical(observation)
  ) {
    fail("provider recovery source changed", 73);
  }
  assertAbsentPredecessors(base, latest);
  if (additionalReassertObservation?.(state) !== undefined) {
    fail("provider recovery physical observer returned a value", 70);
  }
  return held;
}

function assertProviderRecoveryHeld(roots, held) {
  const current = parseCanonical(held.path, "mutation recovery claim");
  const metadata = mutationHeldIdentity(held.path);
  if (!sameMetadata(held.identity, metadata) || !current.bytes.equals(held.bytes)) {
    fail("provider recovery claim identity changed");
  }
  const state = loadState(roots, false);
  if (
    state.mutationLease?.value.action ===
    COLIMA_LIVE_PROVIDER_RESERVATION_ACTION
  ) {
    fail("live provider reservation requires its physical recovery authority", 73);
  }
  const latest = state.mutationRecoveries.at(-1);
  if (latest === undefined || !latest.bytes.equals(held.bytes)) {
    fail("provider recovery claim ownership was refused", 73);
  }
  const currentObservation = providerRecoveryObservation(state);
  const settlement = operationSettlementForSlot(state, state.mutationLease);
  const authorisedSettlementTransition =
    held.observation.observed_settlement_sha256 === ZERO_SHA256 &&
    settlement !== undefined &&
    settlement.value.authority === "recovery" &&
    settlement.value.authority_sha256 === digest(held.bytes) &&
    canonical({
      ...settlementObservation(settlement),
      observed_settlement_sha256: ZERO_SHA256,
    }) === canonical(held.observation) &&
    canonical(currentObservation) === canonical(settlementObservation(settlement));
  if (
    canonical(currentObservation) !== canonical(held.observation) &&
    !authorisedSettlementTransition
  ) {
    fail("provider recovery observation changed", 73);
  }
  return state;
}

function liveProviderReservationRecoveryStageReachable(from, to) {
  const visited = new Set();
  const pending = [from];
  while (pending.length > 0) {
    const current = pending.shift();
    if (current === to) return true;
    if (visited.has(current)) continue;
    visited.add(current);
    for (const next of
      PROVIDER_RESERVATION_RECOVERY_TRANSITIONS[current] ?? []) {
      if (!visited.has(next)) pending.push(next);
    }
  }
  return false;
}

function assertLiveProviderReservationRecoveryHeld(
  roots,
  held,
  argumentsValue,
  fixtureOnly,
) {
  const currentClaim = parseCanonical(
    held.path,
    "live provider reservation recovery claim",
  );
  const claimMetadata = mutationHeldIdentity(held.path);
  if (
    !sameMetadata(held.identity, claimMetadata) ||
    !currentClaim.bytes.equals(held.bytes)
  ) {
    fail("live provider reservation recovery claim identity changed", 73);
  }
  const state = loadState(roots, false);
  const latest = state.mutationRecoveries.at(-1);
  if (
    latest === undefined ||
    !latest.bytes.equals(held.bytes) ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(held.base.lease.bytes)
  ) {
    fail("live provider reservation recovery ownership was refused", 73);
  }
  const { observation, reservation } =
    validateLiveProviderReservationRecoveryPhysicalState(
      state,
      argumentsValue,
      fixtureOnly,
    );
  const previous = held.observation;
  const previousStage = previous.observed_evidence_stage;
  const currentStage = observation.observed_evidence_stage;
  const settlementTransition =
    previous.observed_settlement_sha256 === ZERO_SHA256 &&
    observation.observed_settlement_sha256 !== ZERO_SHA256;
  if (
    !liveProviderReservationRecoveryStageReachable(
      previousStage,
      currentStage,
    ) ||
    (previous.observed_evidence_head_sha256 !== ZERO_SHA256 &&
      previous.observed_evidence_head_sha256 !==
        observation.observed_evidence_head_sha256) ||
    (previous.observed_settlement_sha256 !== ZERO_SHA256 &&
      previous.observed_settlement_sha256 !==
        observation.observed_settlement_sha256) ||
    (settlementTransition &&
      (reservation.settlement?.value.authority !== "recovery" ||
        reservation.settlement.value.authority_sha256 !== digest(held.bytes) ||
        observation.observed_settlement_sha256 !==
          digest(reservation.settlement.bytes)))
  ) {
    fail("live provider reservation recovery observation changed", 73);
  }
  return state;
}

function assertLiveProviderEffectRecoveryHeld(
  roots,
  held,
  argumentsValue,
) {
  const currentClaim = parseCanonical(
    held.path,
    "fixture provider effect recovery claim",
  );
  const claimMetadata = mutationHeldIdentity(held.path);
  if (
    !sameMetadata(held.identity, claimMetadata) ||
    !currentClaim.bytes.equals(held.bytes)
  ) {
    fail("fixture provider effect recovery claim identity changed", 73);
  }
  for (const predecessor of [
    held.base.lease,
    held.predecessor,
  ].filter((entry) => entry !== undefined)) {
    const ownerState = mutationOwnerState(predecessor.value);
    if (ownerState === "current" || ownerState === "unknown") {
      fail(
        "fixture provider effect recovery predecessor became ambiguous",
        73,
      );
    }
  }
  const state = loadState(roots, false);
  const latest = state.mutationRecoveries.at(-1);
  if (
    latest === undefined ||
    !latest.bytes.equals(held.bytes) ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(held.base.lease.bytes)
  ) {
    fail("fixture provider effect recovery ownership was refused", 73);
  }
  const { effect, observation } =
    validateLiveProviderEffectRecoveryPhysicalState(
      state,
      argumentsValue,
    );
  const previousStage = held.observation.observed_evidence_stage;
  const currentStage = observation.observed_evidence_stage;
  const witnessSha256 =
    effect.witness === undefined
      ? held.observation.observed_evidence_head_sha256
      : digest(effect.witness.bytes);
  const previousEventCount = providerEffectEventPrefixCount(
    effect.events,
    held.observation.observed_evidence_head_sha256,
    witnessSha256,
  );
  const currentEventCount = providerEffectEventPrefixCount(
    effect.events,
    observation.observed_evidence_head_sha256,
    witnessSha256,
  );
  const appendedEvents = effect.events.slice(
    previousEventCount,
    currentEventCount,
  );
  if (
    !liveProviderEffectRecoveryStageReachable(previousStage, currentStage) ||
    currentEventCount < previousEventCount ||
    appendedEvents.some(
      (event) =>
        event.value.event_kind !== "completion" ||
        event.value.publisher_authority_sha256 !== digest(held.bytes),
    ) ||
    appendedEvents.length > 1 ||
    state.receiptState.head.sequence !==
      held.base.lease.value.source_sequence ||
    state.receiptState.head_sha256 !==
      held.base.lease.value.source_head_sha256
  ) {
    fail("fixture provider effect recovery observation changed", 73);
  }
  return state;
}

function assertBackgroundCleanupRecoveryHeld(
  roots,
  held,
  { requireExecution = false, requireSource = true } = {},
) {
  const current = parseCanonical(held.path, "mutation recovery claim");
  const metadata = mutationHeldIdentity(held.path);
  if (!sameMetadata(held.identity, metadata) || !current.bytes.equals(held.bytes)) {
    fail("background cleanup recovery claim identity changed");
  }
  const state = loadState(roots, false);
  const latest = state.mutationRecoveries.at(-1);
  if (
    latest === undefined ||
    !latest.bytes.equals(held.bytes) ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(held.base.lease.bytes)
  ) {
    fail("background cleanup recovery ownership was refused", 73);
  }
  return assertBackgroundCleanupStateAuthority(roots, state, {
    requireExecution,
    requireSource,
  });
}

function refreshBackgroundCleanupRecoveryObservation(roots, held) {
  const state = assertBackgroundCleanupRecoveryHeld(roots, held, {
    requireExecution: true,
  });
  const observation = providerRecoveryObservation(state);
  if (canonical(observation) === canonical(held.observation)) return held;
  const latest = state.mutationRecoveries.at(-1);
  if (latest === undefined || !latest.bytes.equals(held.bytes)) {
    fail("background cleanup recovery predecessor changed", 73);
  }
  const sequence = latest.value.sequence + 1;
  if (sequence >= MAX_MUTATION_RECOVERIES) {
    fail("provider mutation recovery capacity was exhausted", 73);
  }
  const owner = currentProcessIdentity();
  const base = held.base;
  const claim = {
    action: base.operation.action,
    chain_root_sha256: recoveryChainRootSha256(
      base.fixtureId,
      base.leaseSha256,
      base.operation,
    ),
    fixture_id: base.fixtureId,
    lease_sha256: base.leaseSha256,
    nonce: randomBytes(16).toString("hex"),
    ...observation,
    operation_contract_sha256: base.operation.operation_contract_sha256,
    operation_kind: base.operation.operation_kind,
    operation_plan_sha256: operationPlanSha256(
      base.operation.operation_plan,
    ),
    owner_boot_sha256: owner.boot_sha256,
    owner_instance_sha256: owner.instance_sha256,
    owner_pid: owner.pid,
    owner_probe: owner.probe,
    parent_sha256: digest(latest.bytes),
    schema: MUTATION_RECOVERY_SCHEMA,
    sequence,
    slot_sequence: base.slotSequence,
    source_head_sha256: state.receiptState.head_sha256,
  };
  const bytes = canonicalBytes(claim);
  const name = recoveryFileName(base.slotSequence, sequence);
  const path = join(state.run, name);
  const reassertClaimPublication = () => {
    const current = assertBackgroundCleanupRecoveryHeld(roots, held, {
      requireExecution: true,
    });
    if (
      canonical(providerRecoveryObservation(current)) !==
      canonical(observation)
    ) {
      fail("background cleanup recovery observation changed", 73);
    }
  };
  if (
    !publishMutationBlocker(state.run, name, bytes, {
      reassertAuthority: reassertClaimPublication,
    })
  ) {
    fail("another provider recovery won the mutation claim", 73);
  }
  const refreshed = Object.freeze({
    base,
    bytes,
    claim,
    identity: mutationHeldIdentity(path),
    observation,
    path,
  });
  const verified = assertBackgroundCleanupRecoveryHeld(roots, refreshed, {
    requireExecution: true,
  });
  if (
    canonical(providerRecoveryObservation(verified)) !==
    canonical(observation)
  ) {
    fail("background cleanup recovery refresh was not durable", 70);
  }
  return refreshed;
}

function backgroundPrefixIsEmpty(prefix) {
  return (
    prefix.evidenceStage === "empty" &&
    prefix.pendingPublication === undefined &&
    prefix.effectFrontier.effect === "empty" &&
    prefix.effectFrontier.disposition === "complete" &&
    prefix.residual.root_disposition === "absent" &&
    prefix.residual.controller_presence === "not-started" &&
    prefix.residual.hostagent_presence === "not-started" &&
    prefix.residual.sockets === "absent" &&
    prefix.residual.private_publications.length === 0
  );
}

function supersededBackgroundReceipt(state, roots) {
  const slot = state.mutationLease;
  if (
    state.pendingPublication?.links !== 1 ||
    slot?.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND
  ) {
    return undefined;
  }
  if (
    state.operationSettlement === undefined &&
    state.receiptState.head.sequence === slot.value.source_sequence &&
    state.receiptState.head_sha256 === slot.value.source_head_sha256 &&
    (preIntentBackgroundResourceCollision(state.providerState.prefix) ||
      !sourceClosureMatches(roots, state.candidate))
  ) {
    return Object.freeze({
      phase: "provider-create-intent",
      result: backgroundProviderIntentResult(slot),
    });
  }
  if (
    state.operationSettlement?.value.disposition === "complete-identity" &&
    state.receiptState.head.phase === "provider-create-intent" &&
    !sourceClosureMatches(roots, state.candidate)
  ) {
    return Object.freeze({
      phase: "provider-create-passed",
      result: backgroundProviderPassedResult(slot, state.operationSettlement),
    });
  }
  return undefined;
}

function discardSupersededBackgroundReceiptPublication(roots, assertAuthority) {
  const state = assertAuthority();
  const pending = state.pendingPublication;
  const slot = state.mutationLease;
  const superseded = supersededBackgroundReceipt(state, roots);
  if (pending === undefined || slot === undefined || superseded === undefined) return state;

  let staged;
  try {
    staged = parseCanonical(pending.path, "pending receipt publication", 1, 0n);
  } catch (error) {
    if (
      error instanceof ClosedFailure &&
      error.message === "pending receipt publication was not canonical JSON"
    ) {
      return state;
    }
    throw error;
  }
  let expected;
  try {
    expected = createNextReceipt(
      state.receipts,
      state.candidate.run_id,
      superseded.phase,
      superseded.result,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) {
      fail("complete pending background receipt publication was refused");
    }
    throw error;
  }
  const expectedBytes = canonicalBytes(expected);
  if (!staged.bytes.equals(expectedBytes)) return state;

  const runMetadata = ownedPrivateDirectory(state.run, "active run state");
  const identity = inspectPendingFile(
    pending.path,
    "pending receipt publication",
    runMetadata.dev,
    new Set([1n]),
  );
  const current = assertAuthority();
  const currentSuperseded = supersededBackgroundReceipt(current, roots);
  if (
    current.pendingPublication?.path !== pending.path ||
    current.pendingPublication.links !== 1 ||
    currentSuperseded === undefined ||
    currentSuperseded.phase !== superseded.phase ||
    canonical(currentSuperseded.result) !== canonical(superseded.result)
  ) {
    fail("pending background receipt retirement authority changed", 73);
  }
  const currentIdentity = inspectPendingFile(
    pending.path,
    "pending receipt publication",
    runMetadata.dev,
    new Set([1n]),
  );
  if (
    !sameMutationArtifact(identity, currentIdentity) ||
    !readPrivate(pending.path, "pending receipt publication", 1, 0n).equals(
      expectedBytes,
    )
  ) {
    fail("pending background receipt publication identity changed");
  }
  try {
    unlinkSync(pending.path);
    syncDirectory(state.run);
  } catch {
    fail("pending background receipt publication retirement failed", 70);
  }

  const verified = assertAuthority();
  if (
    verified.receiptState.head_sha256 === state.receiptState.head_sha256 &&
    verified.receiptState.head.sequence === state.receiptState.head.sequence &&
    verified.pendingPublication === undefined
  ) {
    return verified;
  }
  if (
    verified.receiptState.head_sha256 === digest(expectedBytes) &&
    verified.receiptState.head.phase === superseded.phase &&
    verified.pendingPublication === undefined
  ) {
    return verified;
  }
  fail("pending background receipt publication retirement was not durable", 70);
}

function publishBackgroundOperationSettlement(
  roots,
  state,
  slot,
  { disposition, reassertAuthority, safeCode },
) {
  if (
    typeof reassertAuthority !== "function" ||
    state.mutationLease === undefined ||
    !state.mutationLease.bytes.equals(slot.bytes) ||
    slot.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
    state.operationSettlement !== undefined
  ) {
    fail("background provider settlement authority was refused", 73);
  }
  const recovery = state.mutationRecoveries.at(-1);
  const authority = recovery === undefined ? "owner" : "recovery";
  const authoritySha256 =
    recovery === undefined ? digest(slot.bytes) : digest(recovery.bytes);
  const prefix = state.providerState.prefix;
  const currentDecision = backgroundSettlementDecision(prefix);
  if (
    currentDecision === undefined ||
    currentDecision.disposition !== disposition ||
    currentDecision.safeCode !== safeCode
  ) {
    fail("background provider settlement decision changed", 73);
  }
  const sourceHeadSha256 =
    state.receiptState.head.phase === "provider-create-intent"
      ? state.receiptState.head_sha256
      : state.receiptState.head.sequence === slot.value.source_sequence &&
          state.receiptState.head_sha256 === slot.value.source_head_sha256 &&
          prefix.effectFrontier.effect === "provider-root-collision"
        ? slot.value.source_head_sha256
        : undefined;
  if (sourceHeadSha256 === undefined) {
    fail("background provider settlement source was refused", 73);
  }
  const snapshot = settlementSnapshot(prefix);
  const settlement = {
    authority,
    authority_sha256: authoritySha256,
    ...snapshot,
    disposition,
    fixture_id: slot.value.fixture_id,
    operation_contract_sha256: slot.value.operation_contract_sha256,
    operation_kind: slot.value.operation_kind,
    operation_plan_sha256: operationPlanSha256(slot.value.operation_plan),
    safe_code: safeCode,
    schema: BACKGROUND_CREATE_SETTLEMENT_SCHEMA,
    slot_sequence: slot.value.journal_sequence,
    slot_sha256: digest(slot.bytes),
    source_head_sha256: sourceHeadSha256,
  };
  validateMutationOperationSettlement(settlement, slot, slot.value.fixture_id);
  const bytes = canonicalBytes(settlement);
  const name = mutationOperationFileName(slot.value.journal_sequence);
  const reassertSettlementPublication = () => {
    const current = reassertAuthority();
    if (
      current.mutationLease === undefined ||
      !current.mutationLease.bytes.equals(slot.bytes) ||
      current.operationSettlement !== undefined ||
      current.receiptState.head_sha256 !== sourceHeadSha256 ||
      canonical(settlementSnapshot(current.providerState.prefix)) !== canonical(snapshot)
    ) {
      fail("background provider settlement observation changed", 73);
    }
  };
  if (!publishMutationBlocker(state.run, name, bytes, {
    reassertAuthority: reassertSettlementPublication,
  })) {
    const existing = parseCanonical(join(state.run, name), "mutation operation settlement");
    if (!existing.bytes.equals(bytes)) {
      fail("background provider settlement publication conflicted", 73);
    }
  }
  const verified = loadState(roots, false);
  if (
    verified.operationSettlement === undefined ||
    !verified.operationSettlement.bytes.equals(bytes)
  ) {
    fail("background provider settlement publication was not durable", 70);
  }
  return verified;
}

function closeRecoveredProviderMutation(
  roots,
  held,
  disposition,
  publicationHolds = {},
  operationEvidenceSha256 = ZERO_SHA256,
) {
  const cleanupRecovery =
    held.base.operation.operation_kind ===
    CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND;
  const cleanupAuthorityOptions =
    cleanupRecovery && disposition === "aborted-before-effect"
      ? { requireSource: false }
      : undefined;
  const assertHeld = () =>
    cleanupRecovery
      ? assertBackgroundCleanupRecoveryHeld(
          roots,
          held,
          cleanupAuthorityOptions,
        )
      : assertProviderRecoveryHeld(roots, held);
  assertHeld();
  const receiptAuthority =
    held.base.operation.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND
      ? backgroundReceiptReassertion(
          roots,
          () => assertProviderRecoveryHeld(roots, held),
        )
      : cleanupRecovery
        ? backgroundCleanupReceiptReassertion(roots, assertHeld)
        : assertHeld;
  reconcileReceiptPublication(roots, receiptAuthority);
  reconcileEnvironmentPublication(roots);
  const state = assertHeld();
  return publishMutationClose(
    roots,
    state,
    state.mutationLease,
    disposition,
    assertHeld,
    publicationHolds,
    operationEvidenceSha256,
  );
}

function relinkLiveProviderReservationMarkerForRecovery(
  roots,
  state,
  reservation,
  argumentsValue,
  testCheckpoint,
) {
  const witness = assertLiveProviderReservationRetiredWitness(
    state,
    reservation.publicationPlan,
    argumentsValue,
  );
  if (reservation.settlement !== undefined) {
    fail("settled live provider reservation could not reacquire its marker", 73);
  }
  assertLiveProviderReservationMarkerAbsent(
    state,
    reservation.publicationPlan,
    argumentsValue,
  );
  try {
    linkSync(
      witness.path,
      liveProviderReservationMarkerPath(argumentsValue),
    );
  } catch (error) {
    if (error?.code === "EEXIST") {
      fail("live provider reservation marker was acquired elsewhere", 73);
    }
    fail("live provider reservation marker reacquisition failed", 70);
  }
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-recovery-marker-link",
  );
  syncDirectory(argumentsValue.observationInput.provider_root);
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-recovery-marker-directory-sync",
  );
  const verified = loadState(roots, false);
  assertLiveProviderReservationLinkedTopology(
    verified,
    reservation.publicationPlan,
    argumentsValue,
    2,
  );
  return verified;
}

function closeRecoveredLiveProviderReservation(
  roots,
  held,
  argumentsValue,
  fixtureOnly,
  disposition,
  operationEvidenceSha256,
  testCheckpoint,
) {
  const assertHeld = () =>
    assertLiveProviderReservationRecoveryHeld(
      roots,
      held,
      argumentsValue,
      fixtureOnly,
    );
  const state = assertHeld();
  const reservation = completedLiveProviderReservationForVariant(
    state,
    fixtureOnly,
  );
  if (
    reservation === undefined ||
    (disposition === "aborted-before-effect" &&
      (state.providerReservationStage !== undefined ||
        state.providerReservationWitness !== undefined ||
        reservation.settlement !== undefined)) ||
    (disposition === "completed" &&
      (reservation.settlement === undefined ||
        state.providerReservationStage !== undefined ||
        state.providerReservationWitness?.metadata.nlink !== 1n))
  ) {
    fail("live provider reservation recovery close was refused", 73);
  }
  let afterLinkFailure;
  const closed = publishMutationClose(
    roots,
    state,
    state.mutationLease,
    disposition,
    assertHeld,
    {
      afterLinkObserver:
        testCheckpoint === undefined
          ? undefined
          : () => {
              try {
                callLiveProviderReservationCheckpoint(
                  testCheckpoint,
                  "after-recovery-close-link",
                );
              } catch (error) {
                afterLinkFailure = error;
              }
            },
    },
    operationEvidenceSha256,
  );
  if (afterLinkFailure !== undefined) throw afterLinkFailure;
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-recovery-close",
  );
  return closed;
}

function observeRetiredLiveProviderEffectMarker(
  roots,
  held,
  state,
  effect,
  argumentsValue,
  testCheckpoint,
) {
  assertLiveProviderEffectRetiredWitness(
    state,
    effect.publicationPlan,
    argumentsValue,
  );
  observeLiveProviderEffectPhysicalRoots(
    argumentsValue,
    state,
    effect.source,
    effect.publicationPlan,
    "absent",
  );
  syncDirectory(argumentsValue.observationInput.provider_root);
  callLiveProviderEffectCheckpoint(
    testCheckpoint,
    "after-recovery-marker-retirement-directory-sync",
  );
  const verified = assertLiveProviderEffectRecoveryHeld(
    roots,
    held,
    argumentsValue,
  );
  const verifiedEffect = verified.liveProviderFixtureEffect;
  const witness = assertLiveProviderEffectRetiredWitness(
    verified,
    verifiedEffect.publicationPlan,
    argumentsValue,
  );
  observeLiveProviderEffectPhysicalRoots(
    argumentsValue,
    verified,
    verifiedEffect.source,
    verifiedEffect.publicationPlan,
    "absent",
  );
  const seed = liveProviderEffectBytes({
    marker_witness_identity_sha256:
      witness.value.marker_witness_identity_sha256,
    slot_sha256: digest(verifiedEffect.slot.bytes),
    state: "marker-absent-witness-one-link",
  });
  return Object.freeze({
    markerRetirementObservationSha256: liveProviderEffectDigest(
      Buffer.concat([seed, Buffer.from("marker-retirement", "utf8"), randomBytes(32)]),
    ),
    providerRootFsyncObservationSha256: liveProviderEffectDigest(
      Buffer.concat([seed, Buffer.from("provider-root-fsync", "utf8"), randomBytes(32)]),
    ),
    state: verified,
  });
}

function closeRecoveredLiveProviderEffect(
  roots,
  held,
  argumentsValue,
  disposition,
  operationEvidenceSha256,
  testCheckpoint,
) {
  const assertHeld = () =>
    assertLiveProviderEffectRecoveryHeld(roots, held, argumentsValue);
  const state = assertHeld();
  const effect = state.liveProviderFixtureEffect;
  if (
    effect === undefined ||
    (disposition === "aborted-before-effect" &&
      (!new Set(["not-started", "stage-retired-before-effect"]).has(
        effect.currentStage,
      ) ||
        state.providerEffectStage !== undefined ||
        state.providerEffectWitness !== undefined ||
        state.providerEffectEvents.length !== 0 ||
        operationEvidenceSha256 !== ZERO_SHA256)) ||
    (disposition === "completed" &&
      (effect.currentStage !== "completion-retired" ||
        effect.completion === undefined ||
        state.providerEffectStage !== undefined ||
        state.providerEffectWitness?.metadata.nlink !== 1n ||
        operationEvidenceSha256 !== digest(effect.completion.bytes)))
  ) {
    fail("fixture provider effect recovery close was refused", 73);
  }
  let afterLinkFailure;
  const closed = publishMutationClose(
    roots,
    state,
    state.mutationLease,
    disposition,
    assertHeld,
    {
      afterLinkObserver: () => {
        try {
          callLiveProviderEffectCheckpoint(
            testCheckpoint,
            "after-recovery-close-link",
          );
        } catch (error) {
          afterLinkFailure = error;
        }
      },
    },
    operationEvidenceSha256,
  );
  if (afterLinkFailure !== undefined) throw afterLinkFailure;
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-recovery-close");
  return closed;
}

export function recoverColimaLiveProviderEffectForTest(argumentsValue) {
  const admittedArguments = validateLiveProviderEffectRecoveryArguments(
    argumentsValue,
    { confirmation: true, testCheckpoint: true },
  );
  const { testCheckpoint } = admittedArguments;
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const initial = loadState(roots, false);
  const base = providerRecoveryBase(initial);
  if (
    base.operation.action !== COLIMA_LIVE_PROVIDER_EFFECT_ACTION ||
    liveProviderEffectFixtureOnly(
      base.operation.operation_kind,
      base.operation.operation_contract_sha256,
    ) !== true
  ) {
    fail("fixture provider effect recovery action was refused", 73);
  }
  validateLiveProviderEffectRecoveryPhysicalState(
    initial,
    admittedArguments,
  );
  if (initial.liveProviderFixtureEffect?.currentStage === "attempt-fenced") {
    fail("fixture provider effect attempt fence requires operator resolution", 73);
  }
  const held = acquireProviderRecovery(
    roots,
    admittedArguments.confirmation,
    defaultMutationOwnerProbe,
    {},
    (current) => {
      validateLiveProviderEffectRecoveryPhysicalState(
        current,
        admittedArguments,
      );
    },
    true,
  );
  callLiveProviderEffectCheckpoint(testCheckpoint, "after-recovery-claim");
  let retirement;
  for (let transition = 0; transition < 12; transition += 1) {
    let state = assertLiveProviderEffectRecoveryHeld(
      roots,
      held,
      admittedArguments,
    );
    let effect = state.liveProviderFixtureEffect;
    switch (effect.currentStage) {
      case "not-started":
      case "stage-retired-before-effect": {
        const closed = closeRecoveredLiveProviderEffect(
          roots,
          held,
          admittedArguments,
          "aborted-before-effect",
          ZERO_SHA256,
          testCheckpoint,
        );
        return closed.receiptState.head;
      }
      case "stage-initializing":
      case "stage-only":
        callLiveProviderEffectCheckpoint(
          testCheckpoint,
          "before-recovery-stage-retirement",
        );
        unlinkLiveProviderEffectStage(
          state,
          {
            ...state.providerEffectStage,
            identity: state.providerEffectStage.metadata,
          },
          1,
          testCheckpoint,
        );
        break;
      case "marker-linked": {
        callLiveProviderEffectCheckpoint(
          testCheckpoint,
          "before-recovery-witness-link",
        );
        const witnessPath = join(
          state.run,
          providerEffectWitnessFileName(effect.slot.value.journal_sequence),
        );
        linkLiveProviderEffectWitness(
          state,
          state.providerEffectStage,
          witnessPath,
          testCheckpoint,
        );
        break;
      }
      case "witness-linked":
        callLiveProviderEffectCheckpoint(
          testCheckpoint,
          "before-recovery-stage-unlink",
        );
        unlinkLiveProviderEffectStage(
          state,
          {
            ...state.providerEffectStage,
            identity: state.providerEffectStage.metadata,
          },
          3,
          testCheckpoint,
        );
        break;
      case "marker-held":
      case "authority-held":
        retirement = retireLiveProviderEffectMarker(
          roots,
          state,
          admittedArguments,
          () =>
            assertLiveProviderEffectRecoveryHeld(
              roots,
              held,
              admittedArguments,
            ),
          testCheckpoint,
        );
        break;
      case "witness-unlinked": {
        retirement ??= observeRetiredLiveProviderEffectMarker(
          roots,
          held,
          state,
          effect,
          admittedArguments,
          testCheckpoint,
        );
        state = assertLiveProviderEffectRecoveryHeld(
          roots,
          held,
          admittedArguments,
        );
        effect = state.liveProviderFixtureEffect;
        const completionEvidence =
          liveProviderEffectPreAttemptCompletionEvidence(
            effect,
            digest(held.bytes),
            retirement,
          );
        publishLiveProviderEffectEvent(
          roots,
          state,
          admittedArguments,
          "completion",
          completionEvidence,
          () =>
            assertLiveProviderEffectRecoveryHeld(
              roots,
              held,
              admittedArguments,
            ),
          testCheckpoint,
        );
        break;
      }
      case "completion-retired": {
        const completion = effect.completion;
        const closed = closeRecoveredLiveProviderEffect(
          roots,
          held,
          admittedArguments,
          "completed",
          digest(completion.bytes),
          testCheckpoint,
        );
        return closed.liveProviderFixtureEffect.completion.value;
      }
      case "attempt-fenced":
        fail("fixture provider effect attempt fence requires operator resolution", 73);
      default:
        fail("fixture provider effect recovery stage was refused", 73);
    }
  }
  fail("fixture provider effect recovery did not converge", 73);
}

function recoverColimaLiveProviderReservation(argumentsValue, fixtureOnly) {
  const admittedArguments =
    validateLiveProviderReservationRecoveryArguments(
      argumentsValue,
      fixtureOnly,
      { confirmation: true, testCheckpoint: fixtureOnly },
    );
  const testCheckpoint = fixtureOnly
    ? admittedArguments.testCheckpoint
    : undefined;
  const roots = prepareRoots(
    admittedArguments.repoRoot,
    admittedArguments.stateBase,
    false,
  );
  const initial = loadState(roots, false);
  const base = providerRecoveryBase(initial);
  if (
    base.operation.action !== COLIMA_LIVE_PROVIDER_RESERVATION_ACTION ||
    liveProviderReservationFixtureOnly(
      base.operation.operation_kind,
      base.operation.operation_contract_sha256,
    ) !== fixtureOnly
  ) {
    fail("live provider reservation recovery action was refused", 73);
  }
  validateLiveProviderReservationRecoveryPhysicalState(
    initial,
    admittedArguments,
    fixtureOnly,
  );
  const held = acquireProviderRecovery(
    roots,
    admittedArguments.confirmation,
    defaultMutationOwnerProbe,
    {},
    (current) => {
      validateLiveProviderReservationRecoveryPhysicalState(
        current,
        admittedArguments,
        fixtureOnly,
      );
    },
  );
  callLiveProviderReservationCheckpoint(
    testCheckpoint,
    "after-recovery-claim",
  );
  for (let transition = 0; transition < 12; transition += 1) {
    const state = assertLiveProviderReservationRecoveryHeld(
      roots,
      held,
      admittedArguments,
      fixtureOnly,
    );
    const reservation = completedLiveProviderReservationForVariant(
      state,
      fixtureOnly,
    );
    const observation = liveProviderReservationRecoveryObservation(state);
    switch (observation.observed_evidence_stage) {
      case "not-started":
      case "stage-retired-before-effect": {
        const closed = closeRecoveredLiveProviderReservation(
          roots,
          held,
          admittedArguments,
          fixtureOnly,
          "aborted-before-effect",
          ZERO_SHA256,
          testCheckpoint,
        );
        return closed.receiptState.head;
      }
      case "stage-only":
        callLiveProviderReservationCheckpoint(
          testCheckpoint,
          "before-recovery-stage-retirement",
        );
        unlinkLiveProviderReservationStage(
          state,
          {
            ...state.providerReservationStage,
            identity: state.providerReservationStage.metadata,
          },
          1,
          testCheckpoint,
        );
        break;
      case "marker-linked": {
        callLiveProviderReservationCheckpoint(
          testCheckpoint,
          "before-recovery-witness-link",
        );
        const witnessPath = join(
          state.run,
          providerReservationWitnessFileName(
            reservation.slot.value.journal_sequence,
          ),
        );
        linkLiveProviderReservationWitness(
          state,
          state.providerReservationStage,
          witnessPath,
          testCheckpoint,
        );
        break;
      }
      case "witness-linked":
        callLiveProviderReservationCheckpoint(
          testCheckpoint,
          "before-recovery-stage-unlink",
        );
        unlinkLiveProviderReservationStage(
          state,
          {
            ...state.providerReservationStage,
            identity: state.providerReservationStage.metadata,
          },
          3,
          testCheckpoint,
        );
        break;
      case "witness-unlinked":
        callLiveProviderReservationCheckpoint(
          testCheckpoint,
          "before-recovery-marker-relink",
        );
        relinkLiveProviderReservationMarkerForRecovery(
          roots,
          state,
          reservation,
          admittedArguments,
          testCheckpoint,
        );
        break;
      case "marker-held":
      case "marker-reacquired": {
        callLiveProviderReservationCheckpoint(
          testCheckpoint,
          "before-recovery-retirement-observation",
        );
        assertLiveProviderReservationLinkedTopology(
          state,
          reservation.publicationPlan,
          admittedArguments,
          2,
        );
        const observed = observeLiveProviderReservationPhysicalRoots(
          admittedArguments,
          state,
          reservation.source,
          reservation.publicationPlan,
          fixtureOnly,
          "present",
          undefined,
        );
        assertLiveProviderReservationLinkedTopology(
          loadState(roots, false),
          reservation.publicationPlan,
          admittedArguments,
          2,
        );
        callLiveProviderReservationCheckpoint(
          testCheckpoint,
          "after-recovery-retirement-observation",
        );
        const recoveryAuthority = () =>
          assertLiveProviderReservationRecoveryHeld(
            roots,
            held,
            admittedArguments,
            fixtureOnly,
          );
        publishLiveProviderReservationSettlement(
          roots,
          state,
          reservation,
          reservation.witness.value,
          observed.root_observation,
          recoveryAuthority,
          testCheckpoint,
        );
        break;
      }
      case "retirement-authorized-held":
        retireLiveProviderReservationMarker(
          roots,
          state,
          reservation.publicationPlan,
          admittedArguments,
          testCheckpoint,
        );
        break;
      case "retirement-authorized-retired": {
        const closed = closeRecoveredLiveProviderReservation(
          roots,
          held,
          admittedArguments,
          fixtureOnly,
          "completed",
          digest(reservation.settlement.bytes),
          testCheckpoint,
        );
        const completed = completedLiveProviderReservationForVariant(
          closed,
          fixtureOnly,
        );
        return liveProviderReservationCompletion(completed);
      }
      default:
        fail("live provider reservation recovery stage was refused", 73);
    }
  }
  fail("live provider reservation recovery did not converge", 73);
}

export function recoverColimaLiveProviderReservationForExecutor(
  argumentsValue,
) {
  return recoverColimaLiveProviderReservation(argumentsValue, false);
}

export function recoverColimaLiveProviderReservationForTest(argumentsValue) {
  return recoverColimaLiveProviderReservation(argumentsValue, true);
}

function assertRecoverableLiveProviderStartDecisionState(
  state,
  fixtureOnly,
  message,
) {
  const lease = state.mutationLease;
  const completedIntent = fixtureOnly
    ? state.liveProviderFixtureIntent
    : state.liveProviderIntent;
  const otherIntent = fixtureOnly
    ? state.liveProviderIntent
    : state.liveProviderFixtureIntent;
  const expectedProviderContract = fixtureOnly
    ? "live-provider-fixture-intent-only"
    : "live-provider-intent-only";
  if (
    lease?.value.action !== COLIMA_LIVE_PROVIDER_START_DECISION_ACTION ||
    liveProviderStartDecisionFixtureOnly(
      lease.value.operation_kind,
      lease.value.operation_contract_sha256,
    ) !== fixtureOnly ||
    completedIntent === undefined ||
    otherIntent !== undefined ||
    state.liveProviderStartDecision !== undefined ||
    state.liveProviderFixtureStartDecision !== undefined ||
    state.receiptState.head.sequence !== lease.value.source_sequence ||
    state.receiptState.head_sha256 !== lease.value.source_head_sha256 ||
    lease.value.source_environment_sha256 !== ZERO_SHA256 ||
    lease.value.intent_receipt_sha256 !== ZERO_SHA256 ||
    state.environment !== undefined ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined ||
    state.mutationOperations.length !== 0 ||
    state.operationSettlement !== undefined ||
    state.cleanupSettlement !== undefined ||
    state.providerState.contract !== expectedProviderContract ||
    state.providerState.operationEvidenceSha256 !== ZERO_SHA256 ||
    state.cleanupState.contract !== "journal-only" ||
    state.cleanupState.operationEvidenceSha256 !== ZERO_SHA256
  ) {
    fail(message, 73);
  }
  return state;
}

function recoverLiveProviderStartDecision(
  argumentsValue,
  fixtureOnly,
) {
  exactKeys(
    argumentsValue,
    ["confirmation", "repoRoot", "stateBase"],
    "live provider start decision recovery arguments",
  );
  if (typeof argumentsValue.confirmation !== "string") {
    fail("live provider start decision recovery confirmation was refused", 64);
  }
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  assertRecoverableLiveProviderStartDecisionState(
    loadState(roots, false),
    fixtureOnly,
    "live provider start decision recovery requires an effect-free decision slot",
  );
  const held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
  );
  const state = assertRecoverableLiveProviderStartDecisionState(
    assertProviderRecoveryHeld(roots, held),
    fixtureOnly,
    "live provider start decision recovery observation changed",
  );
  closeRecoveredProviderMutation(roots, held, "aborted-before-effect");
  return state.receiptState.head;
}

export function recoverLiveProviderStartDecisionForExecutor(argumentsValue) {
  return recoverLiveProviderStartDecision(argumentsValue, false);
}

export function recoverLiveProviderStartDecisionForTest(argumentsValue) {
  return recoverLiveProviderStartDecision(argumentsValue, true);
}

export function recoverLiveProviderIntentForExecutor(argumentsValue) {
  exactKeys(
    argumentsValue,
    ["confirmation", "repoRoot", "stateBase"],
    "live provider intent recovery arguments",
  );
  if (typeof argumentsValue.confirmation !== "string") {
    fail("live provider intent recovery confirmation was refused", 64);
  }
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const initial = loadState(roots, false);
  if (
    initial.mutationLease?.value.action !==
      COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
    initial.liveProviderPlan === undefined ||
    initial.liveProviderIntent !== undefined ||
    initial.liveProviderFixtureIntent !== undefined ||
    initial.receiptState.head.sequence !==
      initial.mutationLease.value.source_sequence ||
    initial.receiptState.head_sha256 !==
      initial.mutationLease.value.source_head_sha256 ||
    initial.environment !== undefined ||
    initial.pendingPublication !== undefined ||
    initial.environmentPublication !== undefined ||
    initial.mutationOperations.length !== 0 ||
    initial.providerState.contract !== "live-provider-plan-only" ||
    initial.cleanupState.contract !== "journal-only"
  ) {
    fail("live provider intent recovery requires an effect-free intent slot", 73);
  }
  const held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
  );
  const state = assertProviderRecoveryHeld(roots, held);
  if (
    state.mutationLease?.value.action !==
      COLIMA_LIVE_PROVIDER_INTENT_ACTION ||
    state.liveProviderPlan === undefined ||
    state.liveProviderIntent !== undefined ||
    state.liveProviderFixtureIntent !== undefined ||
    state.receiptState.head.sequence !==
      state.mutationLease.value.source_sequence ||
    state.receiptState.head_sha256 !==
      state.mutationLease.value.source_head_sha256 ||
    state.environment !== undefined ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined ||
    state.mutationOperations.length !== 0 ||
    state.providerState.contract !== "live-provider-plan-only" ||
    state.cleanupState.contract !== "journal-only"
  ) {
    fail("live provider intent recovery observation changed", 73);
  }
  closeRecoveredProviderMutation(roots, held, "aborted-before-effect");
  return state.receiptState.head;
}

export function recoverLiveProviderPlanForExecutor(argumentsValue) {
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const initial = loadState(roots, false);
  if (
    initial.mutationLease?.value.action !== LIVE_PROVIDER_PLAN_ACTION ||
    initial.liveProviderPlan !== undefined ||
    initial.receiptState.head.sequence !==
      initial.mutationLease.value.source_sequence ||
    initial.receiptState.head_sha256 !==
      initial.mutationLease.value.source_head_sha256 ||
    initial.environment !== undefined ||
    initial.providerState.contract !== "synchronous-fake" ||
    initial.cleanupState.contract !== "journal-only"
  ) {
    fail("live provider plan recovery requires an effect-free planning slot", 73);
  }
  const held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
  );
  const state = assertProviderRecoveryHeld(roots, held);
  if (
    state.mutationLease?.value.action !== LIVE_PROVIDER_PLAN_ACTION ||
    state.receiptState.head.sequence !== state.mutationLease.value.source_sequence ||
    state.receiptState.head_sha256 !== state.mutationLease.value.source_head_sha256 ||
    state.environment !== undefined ||
    state.pendingPublication !== undefined ||
    state.environmentPublication !== undefined ||
    state.providerState.contract !== "synchronous-fake" ||
    state.cleanupState.contract !== "journal-only"
  ) {
    fail("live provider plan recovery observation changed", 73);
  }
  closeRecoveredProviderMutation(roots, held, "aborted-before-effect");
  return state.receiptState.head;
}

export function recoverProviderCreateForExecutor(argumentsValue) {
  validateFakeProviderAdapter(argumentsValue.adapter);
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const initial = loadState(roots, false);
  if (
    initial.mutationLease?.value.operation_kind !== DETERMINISTIC_PROVIDER_OPERATION_KIND ||
    initial.providerState.contract !== "synchronous-fake"
  ) {
    fail("provider recovery requires the matching dedicated executor", 73);
  }
  const closePublicationHolds = {
    beforeLinkMilliseconds: argumentsValue.adapter.close_prelink_hold_milliseconds,
  };
  const held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
    {
      afterLinkMilliseconds: argumentsValue.adapter.publication_hold_milliseconds,
      beforeLinkMilliseconds: argumentsValue.adapter.prelink_hold_milliseconds,
    },
  );
  reconcileReceiptPublication(roots);
  let state = assertProviderRecoveryHeld(roots, held);
  const lease = state.mutationLease;
  const head = state.receiptState.head;
  if (head.phase === "plan") {
    if (
      head.sequence !== lease.value.source_sequence ||
      state.receiptState.head_sha256 !== lease.value.source_head_sha256
    ) {
      fail("pre-intent provider recovery binding was refused");
    }
    closeRecoveredProviderMutation(
      roots,
      held,
      "aborted-before-effect",
      closePublicationHolds,
    );
    return head;
  }
  if (head.phase === "provider-create-intent") {
    if (
      state.receiptState.head_sha256 !== lease.value.intent_receipt_sha256 ||
      head.result.provider_contract_sha256 !== FAKE_PROVIDER_CONTRACT_SHA256
    ) {
      fail("provider recovery intent binding was refused");
    }
    if (argumentsValue.adapter.reconcile_outcome === "unknown") {
      fail("provider effect remained uncertain", 73);
    }
    holdFakeProvider(argumentsValue.adapter.reconcile_hold_milliseconds);
    assertProviderRecoveryHeld(roots, held);
    let next = providerAdapterReceipt({
      outcome: argumentsValue.adapter.reconcile_outcome,
      result: argumentsValue.adapter.reconcile_result,
    });
    if (next.phase === "provider-create-passed") {
      let sourceMatches = false;
      try {
        sourceMatches = canonical(sourceClosure(roots.repoRoot)) === canonical(state.candidate.source);
      } catch (error) {
        if (!(error instanceof ClosedFailure)) throw error;
      }
      if (!sourceMatches) {
        next = { phase: "provider-create-failed", result: refusedProviderResult() };
      }
    }
    appendReceiptWithLease(roots, next);
    state = assertProviderRecoveryHeld(roots, held);
  }
  if (state.receiptState.head.phase === "provider-create-passed") {
    let sourceMatches = false;
    try {
      sourceMatches = canonical(sourceClosure(roots.repoRoot)) === canonical(state.candidate.source);
    } catch (error) {
      if (!(error instanceof ClosedFailure)) throw error;
    }
    if (!sourceMatches) {
      appendReceiptWithLease(roots, {
        phase: "execution-failed",
        result: refusedProviderResult(),
      });
      state = assertProviderRecoveryHeld(roots, held);
    }
  }
  if (!new Set(["execution-failed", "provider-create-failed", "provider-create-passed"]).has(
    state.receiptState.head.phase,
  )) {
    fail("provider recovery receipt state was refused");
  }
  closeRecoveredProviderMutation(roots, held, "completed", closePublicationHolds);
  return state.receiptState.head;
}

export function recoverBackgroundProviderCreateForExecutor(argumentsValue) {
  validateBackgroundProviderAdapter(argumentsValue.adapter);
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const initial = loadState(roots, false);
  if (
    initial.mutationLease?.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
    initial.mutationLease.value.operation_contract_sha256 !==
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256 ||
    initial.providerState.contract !== "controlled-background-fake" ||
    initial.mutationLease.value.operation_plan.provider_base.path !==
      argumentsValue.providerBase
  ) {
    fail("provider recovery requires the matching dedicated executor", 73);
  }
  const held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
  );
  const assertAuthority = () => assertProviderRecoveryHeld(roots, held);
  discardSupersededBackgroundReceiptPublication(roots, assertAuthority);
  reconcileReceiptPublication(
    roots,
    backgroundReceiptReassertion(roots, assertAuthority),
  );
  let state = assertAuthority();
  if (
    state.operationSettlement === undefined &&
    state.receiptState.head.sequence === state.mutationLease.value.source_sequence &&
    state.receiptState.head_sha256 === state.mutationLease.value.source_head_sha256 &&
    backgroundPrefixIsEmpty(state.providerState.prefix)
  ) {
    closeRecoveredProviderMutation(roots, held, "aborted-before-effect");
    return state.receiptState.head;
  }
  const decision =
    state.operationSettlement === undefined
      ? backgroundSettlementDecision(state.providerState.prefix)
      : undefined;
  if (state.operationSettlement === undefined && decision === undefined) {
    fail("background provider effect remained uncertain", 73);
  }
  return finishBackgroundProviderMutation({
    adapter: argumentsValue.adapter,
    assertAuthority,
    closeOperation: (settlementSha256) =>
      closeRecoveredProviderMutation(
        roots,
        held,
        "completed",
        {},
        settlementSha256,
      ),
    decision,
    roots,
  });
}

export async function recoverBackgroundProviderCleanupForExecutor(
  argumentsValue,
) {
  validateBackgroundCleanupAdapter(argumentsValue.adapter);
  const roots = prepareRoots(
    argumentsValue.repoRoot,
    argumentsValue.stateBase,
    false,
  );
  const initial = loadState(roots, false);
  if (
    initial.mutationLease?.value.action !== "provider-cleanup" ||
    initial.mutationLease.value.operation_kind !==
      CONTROLLED_BACKGROUND_RETIREMENT_OPERATION_KIND ||
    initial.mutationLease.value.operation_contract_sha256 !==
      CONTROLLED_BACKGROUND_RETIREMENT_CONTRACT_SHA256 ||
    initial.cleanupState.contract !== "controlled-background-retirement"
  ) {
    fail("background cleanup recovery requires the matching dedicated executor", 73);
  }
  let held = acquireProviderRecovery(
    roots,
    argumentsValue.confirmation,
    defaultMutationOwnerProbe,
  );
  holdFakeProvider(argumentsValue.adapter.after_claim_hold_milliseconds);
  const assertAuthority = (options) =>
    assertBackgroundCleanupRecoveryHeld(roots, held, options);
  let state = assertAuthority({ requireSource: false });
  if (
    state.receiptState.head.sequence ===
      state.mutationLease.value.source_sequence &&
    state.receiptState.head_sha256 ===
      state.mutationLease.value.source_head_sha256 &&
    state.pendingPublication === undefined &&
    state.cleanupState.prefix.cleanupStage === "not-started"
  ) {
    closeRecoveredProviderMutation(roots, held, "aborted-before-effect");
    return state.receiptState.head;
  }
  reconcileReceiptPublication(
    roots,
    backgroundCleanupReceiptReassertion(
      roots,
      () => assertAuthority(),
    ),
  );
  state = assertAuthority();
  if (
    state.receiptState.head.phase !== "provider-cleanup-intent" &&
    state.receiptState.head.phase !== "provider-cleanup-passed"
  ) {
    fail("background cleanup recovery receipt state was refused", 73);
  }
  if (state.cleanupState.settlement === undefined) {
    const gate = backgroundCleanupAuthorityGateWithTestObserver(
      backgroundCleanupProcessAuthorityGate(
        roots,
        () => assertAuthority({ requireExecution: true }),
      ),
      argumentsValue.testAuthorityCheckpointObserver,
    );
    if (
      new Set(["not-started", "plan-publication-pending"]).has(
        state.cleanupState.prefix.cleanupStage,
      )
    ) {
      await planControlledBackgroundRetirementWithAuthorityGate(
        {
          bindings: state.cleanupState.bindings,
          evidenceDirectory:
            state.cleanupState.operationPlan.evidence_directory.path,
          fixtureId: state.cleanupState.operationPlan.fixture_id,
          providerBase: state.cleanupState.operationPlan.provider_base.path,
        },
        gate,
      );
    }
    holdFakeProvider(argumentsValue.adapter.after_plan_hold_milliseconds);
    state = assertAuthority();
    const retired = await retireControlledBackgroundProviderWithAuthorityGate(
      {
        crashAfterDeleteSequence:
          argumentsValue.adapter.crash_after_delete_sequence ?? undefined,
        crashAfterDeleteSyscallSequence:
          argumentsValue.adapter.crash_after_delete_syscall_sequence ??
          undefined,
        crashAfterHostagentSettlement:
          argumentsValue.adapter.crash_after_hostagent_settlement,
        evidenceDirectory:
          state.cleanupState.operationPlan.evidence_directory.path,
        fixtureId: state.cleanupState.operationPlan.fixture_id,
        providerBase: state.cleanupState.operationPlan.provider_base.path,
        stopAfterSequence:
          argumentsValue.adapter.stop_after_sequence ?? undefined,
      },
      gate,
    );
    if (retired.complete !== true) {
      fail("background cleanup retirement remained incomplete", 75);
    }
    holdFakeProvider(
      argumentsValue.adapter.after_retirement_hold_milliseconds,
    );
    state = assertAuthority();
    assertBackgroundCleanupComplete(roots, state);
    held = refreshBackgroundCleanupRecoveryObservation(roots, held);
    state = assertAuthority({ requireExecution: true });
    state = publishBackgroundCleanupSettlement(
      roots,
      state,
      state.mutationLease,
      () => assertAuthority(),
    );
  }
  holdFakeProvider(argumentsValue.adapter.after_settlement_hold_milliseconds);
  state = assertAuthority();
  assertBackgroundCleanupComplete(roots, state);
  if (state.receiptState.head.phase === "provider-cleanup-intent") {
    appendReceiptWithLease(
      roots,
      {
        phase: "provider-cleanup-passed",
        result: backgroundCleanupPassedResult(
          state.mutationLease,
          state.cleanupState.settlement,
        ),
      },
      {
        reassertAuthority: backgroundCleanupReceiptReassertion(
          roots,
          () => assertAuthority(),
        ),
      },
    );
    holdFakeProvider(argumentsValue.adapter.after_result_hold_milliseconds);
    state = assertAuthority();
  }
  if (state.receiptState.head.phase !== "provider-cleanup-passed") {
    fail("background cleanup recovery terminal receipt was refused", 73);
  }
  holdFakeProvider(argumentsValue.adapter.before_close_hold_milliseconds);
  closeRecoveredProviderMutation(
    roots,
    held,
    "completed",
    {
      beforeLinkMilliseconds:
        argumentsValue.adapter.close_prelink_hold_milliseconds,
    },
    state.cleanupState.operationEvidenceSha256,
  );
  return state.receiptState.head;
}

export function appendReceiptForExecutor(argumentsValue) {
  if (
    typeof argumentsValue.phase === "string" &&
    (argumentsValue.phase.startsWith("provider-create-") ||
      argumentsValue.phase.startsWith("provider-cleanup-") ||
      argumentsValue.phase === "provider-effect-retired" ||
      argumentsValue.phase === "preflight-refused" ||
      argumentsValue.phase === "finalize-passed")
  ) {
    fail("receipt phase requires its dedicated mutation executor", 64);
  }
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const initial = loadState(roots, false);
  refuseCompletedLiveProviderPlan(initial);
  if (
    initial.providerState.contract === "controlled-background-fake" &&
    typeof argumentsValue.phase === "string" &&
    argumentsValue.phase.startsWith("failure-cleanup-")
  ) {
    fail("background provider failure cleanup requires dedicated ownership evidence", 73);
  }
  if (
    initial.providerState.contract === "controlled-background-fake" &&
    (initial.providerState.createClose?.value.disposition !== "completed" ||
      initial.providerState.settlement?.value.disposition !==
        "complete-identity" ||
      initial.providerState.passedReceipt?.phase !== "provider-create-passed" ||
      initial.cleanupState.contract !== "journal-only")
  ) {
    fail("background provider continuation requires dedicated ownership evidence", 73);
  }
  return withMutationLease(roots, "append-receipt", () =>
    appendReceiptWithLease(roots, argumentsValue),
  );
}

export function appendProviderCleanupReceiptForExecutor(argumentsValue) {
  if (
    typeof argumentsValue.phase !== "string" ||
    !argumentsValue.phase.startsWith("provider-cleanup-")
  ) {
    fail("provider cleanup receipt phase was refused", 64);
  }
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const initial = loadState(roots, false);
  refuseCompletedLiveProviderPlan(initial);
  if (initial.providerState.contract === "controlled-background-fake") {
    fail("background provider cleanup requires dedicated ownership evidence", 73);
  }
  return withMutationLease(roots, "provider-cleanup", () =>
    appendReceiptWithLease(roots, argumentsValue),
  );
}

function appendReceiptWithLease(roots, {
  phase,
  result,
}, { checkSource = true, reassertAuthority } = {}) {
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail("receipt publication authority was refused", 70);
  }
  if (typeof checkSource !== "boolean") {
    fail("receipt publication source check was refused", 70);
  }
  reconcileReceiptPublication(roots, reassertAuthority);
  const sourceRequired =
    !phase.endsWith("-failed") &&
    !phase.startsWith("failure-cleanup-") &&
    phase !== "preflight-refused" &&
      phase !== "project-cleanup-intent" &&
      phase !== "provider-cleanup-intent";
  const state = loadState(roots, sourceRequired && checkSource);
  if (
    !checkSource &&
    (state.mutationLease?.value.operation_kind !== CONTROLLED_BACKGROUND_OPERATION_KIND ||
      phase !== "provider-create-passed")
  ) {
    fail("receipt publication source relaxation was refused", 70);
  }
  if (state.receiptState.head.phase === phase) {
    if (canonical(state.receiptState.head.result) !== canonical(result)) {
      fail("completed receipt result did not match retry");
    }
    return state.receiptState.head;
  }
  if (state.environment !== undefined || state.environmentPublication !== undefined) {
    fail("environment finalization is already in progress", 73);
  }
  if (
    state.receipts.length >= 64 ||
    ((phase.endsWith("-intent") || sourceRequired) && state.receipts.length >= 63)
  ) {
    fail("receipt chain capacity was exhausted", 73);
  }
  let receipt;
  try {
    receipt = createNextReceipt(state.receipts, state.candidate.run_id, phase, result);
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  publishReceipt(state.run, receipt, { reassertAuthority });
  if (sourceRequired && checkSource) {
    let sourceFailure;
    try {
      const current = sourceClosure(roots.repoRoot);
      if (canonical(current) !== canonical(state.candidate.source)) fail("source closure changed");
    } catch (error) {
      if (error instanceof ClosedFailure) sourceFailure = error;
      else throw error;
    }
    if (sourceFailure !== undefined) {
      const drifted = loadState(roots, false);
      if (drifted.receipts.length >= 64) throw sourceFailure;
      if (
        drifted.mutationLease?.value.operation_kind ===
          CONTROLLED_BACKGROUND_OPERATION_KIND &&
        drifted.operationSettlement === undefined
      ) {
        throw sourceFailure;
      }
      let failurePhase = "execution-failed";
      if (phase === "project-cleanup-intent" || phase === "provider-cleanup-intent") {
        failurePhase = phase.replace(/-intent$/, "-failed");
      }
      try {
        const failureReceipt = createNextReceipt(
          drifted.receipts,
          drifted.candidate.run_id,
          failurePhase,
          {
            cleanup_required: true,
            collision_resource: "none",
            resource_disposition: "receipt-owned-or-absent",
            safe_code: "evidence-refused",
          },
        );
        publishReceipt(drifted.run, failureReceipt, { reassertAuthority });
      } catch (error) {
        if (!(error instanceof ReceiptFailure) && !(error instanceof ClosedFailure)) throw error;
      }
      throw sourceFailure;
    }
  }
  const verified = loadState(roots, sourceRequired && checkSource);
  if (verified.receiptState.head_sha256 !== digest(canonicalBytes(receipt))) {
    fail("phase receipt publication was not durable", 70);
  }
  return receipt;
}

function publishReceipt(run, receipt, { reassertAuthority } = {}) {
  const reassertReceiptAuthority =
    reassertAuthority === undefined
      ? undefined
      : () => reassertAuthority(receipt);
  publishPrivateArtifact(
    run,
    RECEIPT_STAGING_NAME,
    receiptFileName(receipt),
    canonicalBytes(receipt),
    "phase receipt",
    { reassertAuthority: reassertReceiptAuthority },
  );
}

function publishPrivateArtifact(
  run,
  stagingName,
  destinationName,
  bytes,
  label,
  { reassertAuthority } = {},
) {
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail(`${label} publication authority was refused`, 70);
  }
  const staging = join(run, stagingName);
  const destination = join(run, destinationName);
  writeExclusive(staging, bytes);
  const runMetadata = ownedPrivateDirectory(run, "active run state");
  const staged = inspectPendingFile(
    staging,
    `${label} staging`,
    runMetadata.dev,
    new Set([1n]),
  );
  try {
    if (reassertAuthority?.() !== undefined) {
      fail(`${label} publication authority returned a value`, 70);
    }
    const current = inspectPendingFile(
      staging,
      `${label} staging`,
      runMetadata.dev,
      new Set([1n]),
    );
    if (
      !sameMutationArtifact(staged, current) ||
      !readPrivate(staging, `${label} staging`, 1, 0n).equals(bytes)
    ) {
      fail(`${label} staging identity changed`);
    }
    linkSync(staging, destination);
    syncDirectory(run);
    unlinkSync(staging);
    syncDirectory(run);
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail(`${label} publication failed`, 70);
  }
}

function reconcileReceiptPublication(roots, reassertAuthority) {
  if (reassertAuthority !== undefined && typeof reassertAuthority !== "function") {
    fail("receipt publication authority was refused", 70);
  }
  const state = loadState(roots, false);
  const pending = state.pendingPublication;
  if (pending === undefined) return;
  if (pending.links === 2) {
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending receipt publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }

  let staged;
  try {
    staged = parseCanonical(pending.path, "pending receipt publication", 1, 0n);
  } catch (error) {
    if (
      !(error instanceof ClosedFailure) ||
      error.message !== "pending receipt publication was not canonical JSON"
    ) {
      throw error;
    }
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending receipt publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }
  let expected;
  const existing = state.receipts[staged.value?.sequence];
  try {
    if (existing !== undefined) {
      expected = existing;
    } else if (staged.value?.phase === "finalize-passed") {
      if (state.environment === undefined) {
        fail("complete final receipt publication requires the environment manifest");
      }
      expected = createFinalization(
        state.candidate,
        state.candidateBytes,
        state.receipts,
      ).receipt;
    } else {
      expected = createNextReceipt(
        state.receipts,
        state.candidate.run_id,
        staged.value?.phase,
        staged.value?.result,
      );
    }
  } catch (error) {
    if (error instanceof ReceiptFailure) {
      fail("complete pending receipt publication was refused");
    }
    throw error;
  }
  if (!staged.bytes.equals(canonicalBytes(expected))) {
    fail("complete pending receipt publication did not match");
  }
  const backgroundReceipt =
    state.mutationLease?.value.operation_kind === CONTROLLED_BACKGROUND_OPERATION_KIND &&
    (expected.phase.startsWith("provider-create-") ||
      expected.phase === "execution-failed" ||
      expected.phase === "preflight-refused");
  if (backgroundReceipt && reassertAuthority === undefined) {
    fail("background provider receipt publication requires current authority", 73);
  }
  if (backgroundReceipt) validateBackgroundProspectiveReceipt(state, expected);
  try {
    if (existing === undefined) {
      if (reassertAuthority?.(expected) !== undefined) {
        fail("receipt publication authority returned a value", 70);
      }
      const current = parseCanonical(
        pending.path,
        "pending receipt publication",
        1,
        0n,
      );
      if (!current.bytes.equals(staged.bytes)) {
        fail("pending receipt publication identity changed");
      }
      linkSync(pending.path, join(state.run, receiptFileName(expected)));
      syncDirectory(state.run);
    }
    unlinkSync(pending.path);
    syncDirectory(state.run);
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail("pending receipt publication recovery failed", 70);
  }
  loadState(roots, false);
}

function reconcileEnvironmentPublication(roots) {
  const state = loadState(roots, false);
  const pending = state.environmentPublication;
  if (pending === undefined) return;
  if (pending.links === 2) {
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending environment publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }

  let staged;
  try {
    staged = parseCanonical(pending.path, "pending environment publication", 1, 0n);
  } catch (error) {
    if (
      !(error instanceof ClosedFailure) ||
      error.message !== "pending environment publication was not canonical JSON"
    ) {
      throw error;
    }
    try {
      unlinkSync(pending.path);
      syncDirectory(state.run);
    } catch {
      fail("pending environment publication recovery failed", 70);
    }
    loadState(roots, false);
    return;
  }
  let finalization;
  try {
    const manifestReceipts =
      state.receiptState.head.phase === "finalize-passed"
        ? state.receipts.slice(0, -1)
        : state.receipts;
    finalization = createFinalization(
      state.candidate,
      state.candidateBytes,
      manifestReceipts,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) {
      fail("complete pending environment publication was refused");
    }
    throw error;
  }
  if (!staged.bytes.equals(finalization.manifestBytes)) {
    fail("complete pending environment publication did not match");
  }
  try {
    if (state.environment === undefined) {
      linkSync(pending.path, join(state.run, ENVIRONMENT_NAME));
      syncDirectory(state.run);
    }
    unlinkSync(pending.path);
    syncDirectory(state.run);
  } catch {
    fail("pending environment publication recovery failed", 70);
  }
  loadState(roots, false);
}

export function finalizeEnvironmentForExecutor(argumentsValue) {
  const roots = prepareRoots(argumentsValue.repoRoot, argumentsValue.stateBase, false);
  const initial = loadState(roots, false);
  refuseCompletedLiveProviderPlan(initial);
  if (initial.providerState.contract === "controlled-background-fake") {
    if (
      initial.cleanupState.contract === "controlled-background-retirement" &&
      initial.cleanupState.close?.value.disposition === "completed" &&
      initial.cleanupState.passedReceipt?.phase === "provider-cleanup-passed"
    ) {
      fail("controlled background evidence is not eligible for environment finalization", 73);
    }
    fail("background provider cleanup must complete before finalization", 73);
  }
  return withMutationLease(roots, "finalize-environment", () =>
    finalizeEnvironmentWithLease(roots),
  );
}

function finalizeEnvironmentWithLease(roots) {
  reconcileReceiptPublication(roots);
  reconcileEnvironmentPublication(roots);
  let state = loadState(roots, false);
  const exactStateEntries = readdirSync(roots.stateBase).sort();
  if (
    JSON.stringify(exactStateEntries) !==
    JSON.stringify([`.run-${state.candidate.run_id}`, "active"].sort())
  ) {
    fail("environment finalization requires absent inert staging");
  }
  let sourceFailure;
  try {
    const current = sourceClosure(roots.repoRoot);
    if (canonical(current) !== canonical(state.candidate.source)) fail("source closure changed");
  } catch (error) {
    if (error instanceof ClosedFailure) sourceFailure = error;
    else throw error;
  }
  if (sourceFailure !== undefined) {
    if (
      state.environment !== undefined &&
      state.receiptState.head.phase !== "finalize-passed"
    ) {
      try {
        unlinkSync(join(state.run, ENVIRONMENT_NAME));
        syncDirectory(state.run);
      } catch {
        fail("stale environment manifest cleanup failed", 70);
      }
      loadState(roots, false);
    }
    throw sourceFailure;
  }
  state = loadState(roots, true);
  if (state.receiptState.head.phase === "finalize-passed") {
    return { manifest: state.environment.value, receipt: state.receiptState.head };
  }
  let finalization;
  try {
    finalization = createFinalization(
      state.candidate,
      state.candidateBytes,
      state.receipts,
    );
  } catch (error) {
    if (error instanceof ReceiptFailure) fail(error.message);
    throw error;
  }
  if (state.environment === undefined) {
    publishPrivateArtifact(
      state.run,
      ENVIRONMENT_STAGING_NAME,
      ENVIRONMENT_NAME,
      finalization.manifestBytes,
      "environment manifest",
    );
  }
  state = loadState(roots, true);
  if (!state.environment.bytes.equals(finalization.manifestBytes)) {
    fail("environment manifest retry did not match");
  }
  publishReceipt(state.run, finalization.receipt);
  const verified = loadState(roots, true);
  if (
    verified.receiptState.head.phase !== "finalize-passed" ||
    verified.receiptState.head_sha256 !== digest(canonicalBytes(finalization.receipt))
  ) {
    fail("environment finalization was not durable", 70);
  }
  return { manifest: verified.environment.value, receipt: verified.receiptState.head };
}

function inspectPendingFile(path, label, expectedDevice, expectedLinks = new Set([1n])) {
  let descriptor;
  let metadata;
  try {
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    metadata = exactFstat(descriptor);
    if (
      !metadata.isFile() ||
      metadata.uid !== OWNER_UID ||
      metadata.dev !== expectedDevice ||
      !expectedLinks.has(metadata.nlink) ||
      (metadata.mode & 0o7777n) !== 0o600n ||
      metadata.size > BigInt(MAX_FILE_BYTES)
    ) {
      fail(`${label} was refused`);
    }
  } catch (error) {
    if (error instanceof ClosedFailure) throw error;
    fail(`${label} was unavailable`, 69);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
  const current = exactLstat(path);
  if (!sameMetadata(metadata, current)) fail(`${label} identity changed`);
  return current;
}

function validateInertStaging(pending) {
  const identity = ownedPrivateDirectory(pending, "pending state");
  const expectedDirectories = new Set(["client", "evidence", "provider", "registry", "runtime"]);
  const expectedFiles = new Set(["00-plan.json", "candidate.json"]);
  for (const entry of readdirSync(pending, { withFileTypes: true })) {
    const path = join(pending, entry.name);
    if (expectedDirectories.has(entry.name)) {
      const metadata = ownedPrivateDirectory(path, "pending plan directory");
      if (metadata.dev !== identity.dev) fail("pending plan crossed a filesystem boundary");
      const children = readdirSync(path);
      if (entry.name === "client") {
        if (
          children.length > 1 ||
          (children.length === 1 && children[0] !== "proxy-template.json")
        ) {
          fail("pending client state was refused");
        }
        if (children.length === 1) {
          inspectPendingFile(join(path, children[0]), "pending proxy template", identity.dev);
        }
      } else if (children.length !== 0) {
        fail("pending external-mutation directory was not empty");
      }
      continue;
    }
    if (expectedFiles.has(entry.name)) {
      inspectPendingFile(path, "pending plan artifact", identity.dev);
      continue;
    }
    fail("pending plan leaf was refused");
  }
  const current = exactLstat(pending);
  if (
    current.isSymbolicLink() ||
    !current.isDirectory() ||
    current.dev !== identity.dev ||
    current.ino !== identity.ino ||
    current.uid !== OWNER_UID
  ) {
    fail("pending plan identity changed during validation");
  }
}

function main() {
  try {
    const { action, values } = parseArgs(process.argv);
    const roots = prepareRoots(values["repo-root"], values["state-base"], action === "plan");
    if (action === "plan") {
      plan(roots, values);
    } else {
      const state = loadState(roots, action === "verify");
      const suffix =
        state.receiptState.head.phase === "plan"
          ? action === "verify" ? "source-verified" : "prepared"
          : `${action === "verify" ? "source-verified" : "prepared"} at ${state.receiptState.head.phase}`;
      process.stdout.write(`clean-engine: plan ${state.candidate.run_id} is ${suffix}\n`);
    }
  } catch (error) {
    if (error instanceof ClosedFailure) {
      process.stderr.write(`clean-engine: ${error.message}\n`);
      process.exit(error.exitStatus);
    }
    process.stderr.write("clean-engine: unexpected closed-state failure\n");
    process.exit(70);
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main();
}
