#!/usr/bin/env node
import { createHash } from "node:crypto";

export const RECEIPT_SCHEMA = "synveda.clean-engine.receipt.v4";
export const ZERO_SHA256 = "0".repeat(64);

const REGISTRY_IMAGE =
  "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33";
const FAILURE_CODES = new Set([
  "child-failed",
  "child-timeout",
  "cleanup-incomplete",
  "evidence-refused",
  "resource-collision",
  "signal",
]);
const SUCCESS_PATH = Object.freeze([
  "plan",
  "provider-create-intent",
  "provider-create-passed",
  "registry-intent",
  "registry-passed",
  "proxy-intent",
  "proxy-passed",
  "builder-canary-intent",
  "builder-canary-passed",
  "compose-browser-intent",
  "compose-browser-passed",
  "project-cleanup-intent",
  "project-cleanup-passed",
  "provider-cleanup-intent",
  "provider-cleanup-passed",
  "finalize-passed",
]);
const INTENT_PHASES = new Set(SUCCESS_PATH.filter((phase) => phase.endsWith("-intent")));
const PASSED_PHASES = new Set(SUCCESS_PATH.filter((phase) => phase.endsWith("-passed")));
const FAILURE_RESULTS = new Set([
  "provider-create-failed",
  "registry-failed",
  "proxy-failed",
  "builder-canary-failed",
  "compose-browser-failed",
  "project-cleanup-failed",
  "provider-cleanup-failed",
  "execution-failed",
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

export class ReceiptFailure extends Error {}

function refuse(message) {
  throw new ReceiptFailure(message);
}

export function canonical(value) {
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
  refuse("unsupported canonical receipt value");
}

export function canonicalBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    refuse(`${label} was malformed`);
  }
  if (JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())) {
    refuse(`${label} fields were refused`);
  }
}

function lowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function privateIpv4Pool(value) {
  if (typeof value !== "string") return false;
  const match = value.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.0\/24$/);
  if (match === null) return false;
  const raw = match.slice(1);
  const octets = raw.map(Number);
  if (octets.some((octet, index) => octet > 255 || String(octet) !== raw[index])) return false;
  const [first, second] = octets;
  return (
    first === 10 ||
    (first === 172 && second >= 16 && second <= 31) ||
    (first === 192 && second === 168)
  );
}

function exactStringArray(value, expected, label) {
  if (JSON.stringify(value) !== JSON.stringify(expected)) refuse(`${label} was refused`);
}

function exactBoolean(value, label) {
  if (value !== true) refuse(`${label} was refused`);
}

function validatePlanResult(result, fixtureId) {
  exactKeys(
    result,
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
    !lowerHex(result.candidate_sha256, 64) ||
    result.project !== `synveda-development-acceptance-${fixtureId.slice(0, 24)}` ||
    result.provider !== "colima" ||
    result.provider_resource !== `synveda-cpr45-${fixtureId}` ||
    typeof result.state_device !== "string" ||
    !/^[0-9]+$/.test(result.state_device) ||
    typeof result.state_inode !== "string" ||
    !/^[0-9]+$/.test(result.state_inode)
  ) {
    refuse("plan result was refused");
  }
}

function validateProviderIntent(result, fixtureId) {
  exactKeys(
    result,
    [
      "operation_kind",
      "operation_plan_sha256",
      "preexisting_resource",
      "provider_contract_sha256",
      "provider_resource",
      "provider_root_key",
    ],
    "provider intent result",
  );
  if (
    !new Set([
      "controlled-background-provider-create-v1",
      "deterministic-fake-provider-create-v1",
    ]).has(result.operation_kind) ||
    !lowerHex(result.operation_plan_sha256, 64) ||
    result.preexisting_resource !== "absent" ||
    !lowerHex(result.provider_contract_sha256, 64) ||
    result.provider_resource !== `synveda-cpr45-${fixtureId}` ||
    (result.operation_kind === "deterministic-fake-provider-create-v1" &&
      (result.operation_plan_sha256 !== ZERO_SHA256 ||
        result.provider_root_key !== `sv-c45-${fixtureId.slice(0, 16)}`)) ||
    (result.operation_kind === "controlled-background-provider-create-v1" &&
      (result.operation_plan_sha256 === ZERO_SHA256 ||
        result.provider_root_key !== `svb-${fixtureId.slice(0, 12)}`))
  ) {
    refuse("provider intent result was refused");
  }
}

function validateProviderPassed(result) {
  if (result?.evidence_class === "deterministic-fixture") {
    exactKeys(
      result,
      [
        "evidence_class",
        "engine_identity_sha256",
        "initial_containers",
        "initial_images",
        "initial_networks",
        "initial_volumes",
        "operation_kind",
        "operation_plan_sha256",
        "platform",
        "provider_contract_sha256",
        "provider_name",
        "provider_version",
        "runtime_client_version",
        "runtime_name",
        "runtime_server_version",
        "socket_contract",
      ],
      "provider result",
    );
    if (
      !lowerHex(result.provider_contract_sha256, 64) ||
      !lowerHex(result.engine_identity_sha256, 64) ||
      result.provider_name !== "colima" ||
      result.provider_version !== "0.10.3" ||
      result.runtime_name !== "docker" ||
      result.runtime_client_version !== "29.4.0" ||
      result.runtime_server_version !== "29.4.0" ||
      result.platform !== "darwin-arm64-colima-vz" ||
      result.operation_kind !== "deterministic-fake-provider-create-v1" ||
      result.operation_plan_sha256 !== ZERO_SHA256 ||
      result.initial_containers !== 0 ||
      result.initial_images !== 0 ||
      result.initial_volumes !== 0 ||
      result.socket_contract !== "receipt-owned-unix"
    ) {
      refuse("provider result was refused");
    }
    exactStringArray(result.initial_networks, ["bridge", "host", "none"], "provider networks");
    return;
  }
  if (result?.evidence_class === "controlled-background-fake") {
    exactKeys(
      result,
      [
        "evidence_class",
        "operation_evidence_sha256",
        "operation_kind",
        "operation_plan_sha256",
        "platform",
        "provider_contract_sha256",
        "provider_name",
        "runtime_name",
      ],
      "provider result",
    );
    if (
      !lowerHex(result.provider_contract_sha256, 64) ||
      !lowerHex(result.operation_evidence_sha256, 64) ||
      result.operation_evidence_sha256 === ZERO_SHA256 ||
      result.operation_kind !== "controlled-background-provider-create-v1" ||
      !lowerHex(result.operation_plan_sha256, 64) ||
      result.provider_name !== "controlled-background-fake" ||
      result.runtime_name !== "docker-fake" ||
      result.platform !== "deterministic-posix"
    ) {
      refuse("provider result was refused");
    }
    return;
  }
  refuse("provider result was refused");
}

function validateRegistryIntent(result, fixtureId) {
  exactKeys(
    result,
    ["authentication", "container", "image", "port", "transport"],
    "registry intent result",
  );
  if (
    result.authentication !== "basic-bcrypt-cost-12" ||
    result.container !== `synveda-cpr45-registry-${fixtureId.slice(0, 16)}` ||
    result.image !== REGISTRY_IMAGE ||
    !Number.isSafeInteger(result.port) ||
    result.port < 49_152 ||
    result.port > 65_535 ||
    result.transport !== "tls-loopback"
  ) {
    refuse("registry intent result was refused");
  }
}

function validateRegistryPassed(result) {
  exactKeys(
    result,
    [
      "authenticated_pull",
      "authenticated_push",
      "basic_challenge",
      "canary_image_sha256",
      "certificate_sha256",
      "negative_status",
      "unauthenticated_pull_rejected",
      "wrong_password_rejected",
    ],
    "registry result",
  );
  if (
    !lowerHex(result.certificate_sha256, 64) ||
    !lowerHex(result.canary_image_sha256, 64) ||
    result.negative_status !== 401
  ) {
    refuse("registry result was refused");
  }
  for (const key of [
    "authenticated_pull",
    "authenticated_push",
    "basic_challenge",
    "unauthenticated_pull_rejected",
    "wrong_password_rejected",
  ]) {
    exactBoolean(result[key], `registry ${key}`);
  }
}

function validateProxyIntent(result) {
  exactKeys(
    result,
    ["config", "expected_injected_variables", "expected_runtime_empty_variables"],
    "proxy intent result",
  );
  if (
    result.config !== "synthetic-nonsecret-v1" ||
    result.expected_injected_variables !== 10 ||
    result.expected_runtime_empty_variables !== 10
  ) {
    refuse("proxy intent result was refused");
  }
}

function validateProxyPassed(result) {
  exactKeys(
    result,
    ["auth_preserved", "injected_variables", "runtime_empty_variables"],
    "proxy result",
  );
  if (result.injected_variables !== 10 || result.runtime_empty_variables !== 10) {
    refuse("proxy result was refused");
  }
  exactBoolean(result.auth_preserved, "proxy authentication preservation");
}

function validateBuilderIntent(result, fixtureId) {
  exactKeys(
    result,
    ["builder", "canonical_builder", "endpoint", "expected_connections"],
    "builder intent result",
  );
  if (
    result.builder !== `synveda-cpr45-canary-${fixtureId.slice(0, 16)}` ||
    result.canonical_builder !== "default" ||
    result.endpoint !== "loopback-inert-tcp" ||
    result.expected_connections !== 0
  ) {
    refuse("builder intent result was refused");
  }
}

function validateBuilderPassed(result) {
  exactKeys(
    result,
    [
      "canonical_builder_driver",
      "canonical_builder_endpoint",
      "connections",
      "private_buildx_removed",
    ],
    "builder result",
  );
  if (
    result.canonical_builder_driver !== "docker" ||
    result.canonical_builder_endpoint !== "default" ||
    result.connections !== 0
  ) {
    refuse("builder result was refused");
  }
  exactBoolean(result.private_buildx_removed, "private Buildx cleanup");
}

function validateBrowserIntent(result, fixtureId) {
  exactKeys(result, ["capture", "profiles", "project"], "browser intent result");
  if (
    result.capture !== "disabled" ||
    result.project !== `synveda-development-acceptance-${fixtureId.slice(0, 24)}`
  ) {
    refuse("browser intent result was refused");
  }
  exactStringArray(result.profiles, ["browser-acceptance", "demo"], "browser profiles");
}

function validateBrowserPassed(result) {
  exactKeys(
    result,
    [
      "admin_admitted",
      "browser_exit",
      "captured_artifacts",
      "container_proxy_empty_variables",
      "logout",
      "pkce_s256",
    ],
    "browser result",
  );
  if (
    result.browser_exit !== 0 ||
    result.captured_artifacts !== 0 ||
    result.container_proxy_empty_variables !== 10
  ) {
    refuse("browser result was refused");
  }
  for (const key of ["admin_admitted", "logout", "pkce_s256"]) {
    exactBoolean(result[key], `browser ${key}`);
  }
}

function validateProjectCleanupIntent(result, fixtureId) {
  exactKeys(result, ["project", "resolver", "scope"], "project cleanup intent result");
  if (
    result.project !== `synveda-development-acceptance-${fixtureId.slice(0, 24)}` ||
    result.resolver !== "managed-test-block" ||
    result.scope !== "exact-receipt-owned-only"
  ) {
    refuse("project cleanup intent result was refused");
  }
}

function validateProjectCleanupPassed(result) {
  exactKeys(
    result,
    [
      "builder_canary_absent",
      "project_absent",
      "registry_absent",
      "resolver_absent",
      "runtime_secrets_absent",
    ],
    "project cleanup result",
  );
  for (const value of Object.values(result)) exactBoolean(value, "project cleanup assertion");
}

function validateProviderCleanupIntent(result, fixtureId) {
  exactKeys(result, ["command", "provider_resource", "scope"], "provider cleanup intent result");
  if (
    result.command !== "colima-delete-data-force" ||
    result.provider_resource !== `synveda-cpr45-${fixtureId}` ||
    result.scope !== "exact-receipt-owned-only"
  ) {
    refuse("provider cleanup intent result was refused");
  }
}

function validateProviderCleanupPassed(result) {
  exactKeys(
    result,
    [
      "context_absent",
      "inert_staging_absent",
      "provider_absent",
      "runtime_root_absent",
      "socket_absent",
      "source_closure_unchanged",
    ],
    "provider cleanup result",
  );
  for (const value of Object.values(result)) exactBoolean(value, "provider cleanup assertion");
}

function validateFinalizePassed(result, previousSha256) {
  exactKeys(
    result,
    ["assertion_count", "environment_manifest_sha256", "receipt_head_sha256"],
    "finalize result",
  );
  if (
    result.assertion_count !== 10 ||
    !lowerHex(result.environment_manifest_sha256, 64) ||
    result.receipt_head_sha256 !== previousSha256
  ) {
    refuse("finalize result was refused");
  }
}

function validateCollisionDisposition(result, allowedCollisions, label) {
  const collision = result.safe_code === "resource-collision";
  if (
    collision !== (result.resource_disposition === "foreign-preserved") ||
    collision !== (result.collision_resource !== "none") ||
    (collision && !allowedCollisions.includes(result.collision_resource))
  ) {
    refuse(`${label} collision was refused`);
  }
}

function liveAuthorizedResources(receipts) {
  const resources = new Set();
  for (const receipt of receipts) {
    switch (receipt.phase) {
      case "provider-create-intent":
        resources.add("provider");
        break;
      case "registry-intent":
        resources.add("registry");
        resources.add("runtime-secrets");
        break;
      case "proxy-intent":
        resources.add("runtime-secrets");
        break;
      case "builder-canary-intent":
        resources.add("builder-canary");
        break;
      case "builder-canary-passed":
        resources.delete("builder-canary");
        break;
      case "compose-browser-intent":
        resources.add("compose-project");
        resources.add("resolver");
        resources.add("runtime-secrets");
        break;
      case "project-cleanup-passed":
        for (const resource of [
          "builder-canary",
          "compose-project",
          "registry",
          "resolver",
          "runtime-secrets",
        ]) {
          resources.delete(resource);
        }
        break;
      case "provider-cleanup-passed":
        resources.delete("provider");
        break;
      case "failure-cleanup-passed":
        resources.clear();
        break;
      default:
        break;
    }
    if (receipt.result?.resource_disposition === "foreign-preserved") {
      resources.delete(receipt.result.collision_resource);
    }
  }
  return [...resources].sort();
}

function collisionResourcesForPhase(phase, receipts) {
  const live = new Set(liveAuthorizedResources(receipts));
  const phaseResources = {
    "builder-canary-failed": ["builder-canary"],
    "compose-browser-failed": ["compose-project", "resolver", "runtime-secrets"],
    "project-cleanup-failed": [
      "builder-canary",
      "compose-project",
      "registry",
      "resolver",
      "runtime-secrets",
    ],
    "provider-cleanup-failed": ["provider"],
    "provider-create-failed": ["provider"],
    "proxy-failed": ["runtime-secrets"],
    "registry-failed": ["registry", "runtime-secrets"],
  }[phase];
  return (phaseResources ?? []).filter((resource) => live.has(resource)).sort();
}

function validateFailureResult(result, phase, receipts) {
  exactKeys(
    result,
    ["cleanup_required", "collision_resource", "resource_disposition", "safe_code"],
    "failure result",
  );
  if (
    result.cleanup_required !== true ||
    !FAILURE_CODES.has(result.safe_code) ||
    !new Set(["foreign-preserved", "receipt-owned-or-absent"]).has(
      result.resource_disposition,
    ) ||
    !new Set([
      "builder-canary",
      "compose-project",
      "none",
      "provider",
      "registry",
      "resolver",
      "runtime-secrets",
    ]).has(result.collision_resource)
  ) {
    refuse("failure result was refused");
  }
  validateCollisionDisposition(
    result,
    collisionResourcesForPhase(phase, receipts),
    "failure result",
  );
}

function authorizedResources(receipts) {
  return liveAuthorizedResources(receipts);
}

function validateFailureCleanupIntent(result, receipts) {
  exactKeys(
    result,
    ["authorized_resources", "scope"],
    "failure cleanup intent result",
  );
  if (
    result.scope !== "exact-receipt-owned-only" ||
    JSON.stringify(result.authorized_resources) !== JSON.stringify(authorizedResources(receipts))
  ) {
    refuse("failure cleanup intent result was refused");
  }
}

function validateFailureCleanupPassed(result) {
  exactKeys(
    result,
    ["foreign_collision_preserved", "manifest_published", "receipt_owned_resources_absent"],
    "failure cleanup result",
  );
  if (result.manifest_published !== false) refuse("failure cleanup manifest state was refused");
  exactBoolean(result.foreign_collision_preserved, "failure cleanup collision preservation");
  exactBoolean(result.receipt_owned_resources_absent, "failure cleanup resource absence");
}

function validatePreflightRefused(result) {
  exactKeys(
    result,
    ["cleanup_required", "collision_resource", "resource_disposition", "safe_code"],
    "preflight refusal result",
  );
  if (
    result.cleanup_required !== false ||
    result.safe_code !== "resource-collision" ||
    result.collision_resource !== "provider" ||
    result.resource_disposition !== "foreign-preserved"
  ) {
    refuse("preflight refusal result was refused");
  }
}

function validateFailureCleanupFailed(result, receipts) {
  exactKeys(
    result,
    [
      "cleanup_incomplete",
      "collision_resource",
      "resource_disposition",
      "safe_code",
    ],
    "failed cleanup result",
  );
  if (
    result.cleanup_incomplete !== true ||
    !FAILURE_CODES.has(result.safe_code) ||
    !new Set(["foreign-preserved", "receipt-owned-or-absent"]).has(
      result.resource_disposition,
    ) ||
    !new Set([
      "builder-canary",
      "compose-project",
      "none",
      "provider",
      "registry",
      "resolver",
      "runtime-secrets",
    ]).has(result.collision_resource)
  ) {
    refuse("failed cleanup result was refused");
  }
  validateCollisionDisposition(
    result,
    liveAuthorizedResources(receipts),
    "failed cleanup result",
  );
}

function validateResult(phase, result, fixtureId, previousSha256, previousReceipts) {
  switch (phase) {
    case "plan": return validatePlanResult(result, fixtureId);
    case "provider-create-intent": return validateProviderIntent(result, fixtureId);
    case "provider-create-passed": return validateProviderPassed(result);
    case "registry-intent": return validateRegistryIntent(result, fixtureId);
    case "registry-passed": return validateRegistryPassed(result);
    case "proxy-intent": return validateProxyIntent(result);
    case "proxy-passed": return validateProxyPassed(result);
    case "builder-canary-intent": return validateBuilderIntent(result, fixtureId);
    case "builder-canary-passed": return validateBuilderPassed(result);
    case "compose-browser-intent": return validateBrowserIntent(result, fixtureId);
    case "compose-browser-passed": return validateBrowserPassed(result);
    case "project-cleanup-intent": return validateProjectCleanupIntent(result, fixtureId);
    case "project-cleanup-passed": return validateProjectCleanupPassed(result);
    case "provider-cleanup-intent": return validateProviderCleanupIntent(result, fixtureId);
    case "provider-cleanup-passed": return validateProviderCleanupPassed(result);
    case "finalize-passed": return validateFinalizePassed(result, previousSha256);
    case "failure-cleanup-intent": return validateFailureCleanupIntent(result, previousReceipts);
    case "failure-cleanup-passed": return validateFailureCleanupPassed(result);
    case "failure-cleanup-failed": return validateFailureCleanupFailed(result, previousReceipts);
    case "preflight-refused": return validatePreflightRefused(result);
    default:
      if (FAILURE_RESULTS.has(phase)) {
        return validateFailureResult(result, phase, previousReceipts);
      }
      refuse("receipt phase was refused");
  }
}

function expectedOutcome(phase) {
  if (phase === "plan" || PASSED_PHASES.has(phase) || phase === "failure-cleanup-passed") {
    return "passed";
  }
  if (INTENT_PHASES.has(phase) || phase === "failure-cleanup-intent") return "intent";
  if (
    FAILURE_RESULTS.has(phase) ||
    phase === "failure-cleanup-failed" ||
    phase === "preflight-refused"
  ) return "failed";
  refuse("receipt phase was refused");
}

function allowedNext(previousPhase) {
  if (previousPhase === "plan") return ["provider-create-intent", "preflight-refused"];
  const successIndex = SUCCESS_PATH.indexOf(previousPhase);
  if (successIndex >= 0 && successIndex < SUCCESS_PATH.length - 1) {
    const next = [SUCCESS_PATH[successIndex + 1]];
    if (previousPhase.endsWith("-intent")) {
      next.push(previousPhase.replace(/-intent$/, "-failed"));
    }
    if (!new Set(["project-cleanup-intent", "provider-cleanup-intent"]).has(previousPhase)) {
      next.push("execution-failed");
    }
    return [...new Set(next)];
  }
  if (FAILURE_RESULTS.has(previousPhase)) return ["failure-cleanup-intent"];
  if (previousPhase === "failure-cleanup-intent") {
    return ["failure-cleanup-passed", "failure-cleanup-failed"];
  }
  if (previousPhase === "failure-cleanup-failed") return ["failure-cleanup-intent"];
  return [];
}

export function receiptFileName(receipt) {
  return `${String(receipt.sequence).padStart(2, "0")}-${receipt.phase}.json`;
}

export function validateReceiptChain(receipts, fixtureId) {
  if (!lowerHex(fixtureId, 32) || !Array.isArray(receipts) || receipts.length < 1) {
    refuse("receipt chain was malformed");
  }
  let previousBytes;
  for (let index = 0; index < receipts.length; index += 1) {
    const receipt = receipts[index];
    exactKeys(
      receipt,
      ["fixture_id", "outcome", "phase", "previous_sha256", "result", "schema", "sequence"],
      "receipt",
    );
    const expectedPrevious = index === 0 ? ZERO_SHA256 : sha256(previousBytes);
    if (
      receipt.schema !== RECEIPT_SCHEMA ||
      receipt.fixture_id !== fixtureId ||
      receipt.sequence !== index ||
      receipt.previous_sha256 !== expectedPrevious ||
      receipt.outcome !== expectedOutcome(receipt.phase) ||
      (index === 0 && receipt.phase !== "plan") ||
      (index > 0 && !allowedNext(receipts[index - 1].phase).includes(receipt.phase))
    ) {
      refuse("receipt chain was refused");
    }
    validateResult(
      receipt.phase,
      receipt.result,
      fixtureId,
      expectedPrevious,
      receipts.slice(0, index),
    );
    previousBytes = canonicalBytes(receipt);
  }
  const providerIntent = receipts.find((receipt) => receipt.phase === "provider-create-intent");
  const providerPassed = receipts.find((receipt) => receipt.phase === "provider-create-passed");
  if (
    providerPassed !== undefined &&
    (providerPassed.result.provider_contract_sha256 !==
      providerIntent?.result.provider_contract_sha256 ||
      providerPassed.result.operation_kind !== providerIntent?.result.operation_kind ||
      providerPassed.result.operation_plan_sha256 !==
        providerIntent?.result.operation_plan_sha256)
  ) {
    refuse("provider result contract binding was refused");
  }
  return Object.freeze({
    head: receipts.at(-1),
    head_sha256: sha256(previousBytes),
    manifest_eligible:
      receipts.length === SUCCESS_PATH.length - 1 &&
      receipts.every((receipt, index) => receipt.phase === SUCCESS_PATH[index]) &&
      providerPassed?.result.evidence_class === "deterministic-fixture",
    terminal:
      receipts.at(-1).phase === "finalize-passed" ||
      receipts.at(-1).phase === "failure-cleanup-passed" ||
      receipts.at(-1).phase === "preflight-refused",
  });
}

function constructNextReceipt(receipts, fixtureId, phase, result) {
  const state = validateReceiptChain(receipts, fixtureId);
  if (!allowedNext(state.head.phase).includes(phase)) refuse("next receipt phase was refused");
  const receipt = {
    fixture_id: fixtureId,
    outcome: expectedOutcome(phase),
    phase,
    previous_sha256: state.head_sha256,
    result,
    schema: RECEIPT_SCHEMA,
    sequence: receipts.length,
  };
  validateReceiptChain([...receipts, receipt], fixtureId);
  return receipt;
}

export function createNextReceipt(receipts, fixtureId, phase, result) {
  if (phase === "finalize-passed") refuse("finalization must be state-owned");
  return constructNextReceipt(receipts, fixtureId, phase, result);
}

function validateManifestCandidate(candidate) {
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
    "environment manifest candidate",
  );
  const createdAt =
    typeof candidate.created_at === "string" ? Date.parse(candidate.created_at) : Number.NaN;
  if (
    candidate.kind !== "synveda-cpr45-clean-engine-candidate" ||
    candidate.feature !== "CPR-45" ||
    candidate.schema_version !== 1 ||
    !lowerHex(candidate.run_id, 32) ||
    typeof candidate.created_at !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(candidate.created_at) ||
    !Number.isFinite(createdAt) ||
    new Date(createdAt).toISOString() !== candidate.created_at
  ) {
    refuse("environment manifest candidate was refused");
  }
  exactStringArray(candidate.requested_assertions, REQUESTED_ASSERTIONS, "candidate assertions");
  exactStringArray(candidate.excluded_claims, EXCLUDED_CLAIMS, "candidate excluded claims");
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
    candidate.fixtures.builder_canary !== "ambient-remote-inert-zero-read-v1" ||
    candidate.fixtures.docker_proxy !== "synthetic-nonsecret-v1" ||
    candidate.fixtures.registry_authentication !== "one-run-basic-bcrypt" ||
    candidate.fixtures.registry_image !== REGISTRY_IMAGE ||
    candidate.fixtures.registry_transport !== "loopback-tls-ephemeral"
  ) {
    refuse("candidate fixtures were refused");
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
    candidate.selection.app_host !== "app.synveda.test" ||
    candidate.selection.auth_host !== "auth.synveda.test" ||
    candidate.selection.oidc !== "bundled" ||
    candidate.selection.port !== 8080 ||
    candidate.selection.postgres !== "bundled" ||
    candidate.selection.project !== `synveda-development-${suffix}` ||
    candidate.selection.project_suffix !== suffix ||
    candidate.selection.runtime !== "development" ||
    candidate.selection.scheme !== "http" ||
    !privateIpv4Pool(candidate.selection.ipv4_pool)
  ) {
    refuse("candidate selection was refused");
  }
  exactStringArray(
    candidate.selection.profiles,
    ["browser-acceptance", "demo"],
    "candidate profiles",
  );
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
  for (const key of [
    "build_context_manifest_sha256",
    "deployment_contract_sha256",
    "deployment_input_manifest_sha256",
    "tracked_index_manifest_sha256",
  ]) {
    if (!lowerHex(candidate.source[key], 64)) refuse("candidate source digest was refused");
  }
  if (
    !lowerHex(candidate.source.commit_sha, 40) ||
    !lowerHex(candidate.source.tree_sha, 40) ||
    candidate.source.worktree_clean !== true
  ) {
    refuse("candidate source was refused");
  }
}

export function buildEnvironmentManifest(candidate, candidateBytes, receipts) {
  validateManifestCandidate(candidate);
  const state = validateReceiptChain(receipts, candidate?.run_id);
  if (!state.manifest_eligible) refuse("environment manifest is not eligible");
  if (!Buffer.isBuffer(candidateBytes) || !canonicalBytes(candidate).equals(candidateBytes)) {
    refuse("environment manifest candidate was refused");
  }
  if (sha256(candidateBytes) !== receipts[0].result.candidate_sha256) {
    refuse("environment manifest candidate digest was refused");
  }
  const byPhase = new Map(receipts.map((receipt) => [receipt.phase, receipt]));
  const evidence = (phase) => {
    const receipt = byPhase.get(phase);
    return { receipt_sequence: receipt.sequence, receipt_sha256: sha256(canonicalBytes(receipt)) };
  };
  const assertions = {
    "browser-pkce-admin-logout-no-capture": evidence("compose-browser-passed"),
    "builder-canary-zero-connections": evidence("builder-canary-passed"),
    "canonical-proxy-values-empty": evidence("proxy-passed"),
    "clean-engine-initial-state": evidence("provider-create-passed"),
    "disposable-engine-destroyed": evidence("provider-cleanup-passed"),
    "docker-client-proxy-active": evidence("proxy-passed"),
    "exact-local-embedded-builder": evidence("builder-canary-passed"),
    "exact-project-cleanup": evidence("project-cleanup-passed"),
    "registry-auth-negative-positive": evidence("registry-passed"),
    "source-closure-unchanged": evidence("provider-cleanup-passed"),
  };
  if (canonical(Object.keys(assertions).sort()) !== canonical([...candidate.requested_assertions].sort())) {
    refuse("environment assertion vocabulary was refused");
  }
  const provider = byPhase.get("provider-create-passed").result;
  const registry = byPhase.get("registry-intent").result;
  return {
    assertions,
    candidate_sha256: sha256(candidateBytes),
    cleanup: {
      builder_canary_absent: true,
      inert_staging_absent: true,
      project_absent: true,
      provider_absent: true,
      registry_absent: true,
      resolver_absent: true,
      runtime_root_absent: true,
      runtime_secrets_absent: true,
    },
    environment: {
      evidence_class: provider.evidence_class,
      platform: provider.platform,
      provider_contract_sha256: provider.provider_contract_sha256,
      provider_name: provider.provider_name,
      provider_version: provider.provider_version,
      registry_authentication: registry.authentication,
      registry_image: registry.image,
      registry_transport: registry.transport,
      runtime_client_version: provider.runtime_client_version,
      runtime_name: provider.runtime_name,
      runtime_server_version: provider.runtime_server_version,
    },
    excluded_claims: candidate.excluded_claims,
    feature: "CPR-45",
    receipt: { count: receipts.length, head_sha256: state.head_sha256 },
    run_id: candidate.run_id,
    schema: "synveda.clean-engine.synthetic-environment.v1",
    selection: candidate.selection,
    source: candidate.source,
  };
}

export function createFinalization(candidate, candidateBytes, receipts) {
  const manifest = buildEnvironmentManifest(candidate, candidateBytes, receipts);
  const manifestBytes = canonicalBytes(manifest);
  const state = validateReceiptChain(receipts, candidate.run_id);
  const receipt = constructNextReceipt(receipts, candidate.run_id, "finalize-passed", {
    assertion_count: REQUESTED_ASSERTIONS.length,
    environment_manifest_sha256: sha256(manifestBytes),
    receipt_head_sha256: state.head_sha256,
  });
  return { manifest, manifestBytes, receipt };
}

export const receiptSuccessPath = SUCCESS_PATH;
