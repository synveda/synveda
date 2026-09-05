#!/usr/bin/env node
import { createHash, timingSafeEqual } from "node:crypto";
import {
  COLIMA_LIVE_OBSERVATION_SCHEMA,
  COLIMA_LIVE_REQUIREMENTS_SHA256,
} from "./clean-engine-colima-live-contract.mjs";

export const PROVIDER_ADAPTER_REGISTRY_SCHEMA =
  "synveda.clean-engine.provider-adapter-registry.v1";
export const PROVIDER_ADAPTER_KEY_SCHEMA =
  "synveda.clean-engine.provider-adapter-key.v1";
export const PROVIDER_ADAPTER_RESOLUTION_SCHEMA =
  "synveda.clean-engine.provider-adapter-resolution.v1";
export const COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-create-operation-contract.v1";
export const COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-cleanup-operation-contract.v1";
export const COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA =
  "synveda.clean-engine.colima-live-create-evidence.v1";
export const COLIMA_LIVE_CLEANUP_EVIDENCE_SCHEMA =
  "synveda.clean-engine.colima-live-cleanup-evidence.v1";
export const COLIMA_LIVE_CREATE_OPERATION_KIND =
  "colima-vz-docker-live-create-v1";
export const COLIMA_LIVE_CLEANUP_OPERATION_KIND =
  "colima-vz-docker-live-cleanup-v1";
export const COLIMA_LIVE_PROVIDER_CLASS = "colima-vz-docker-live";

const ADAPTER_ID = "colima-vz-docker-live-plan-only-v1";
const TUPLE_FIELDS = Object.freeze([
  "action",
  "operation_contract_sha256",
  "operation_kind",
  "provider_class",
]);

export class ProviderAdapterRegistryFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new ProviderAdapterRegistryFailure(message, exitStatus);
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
  fail("provider adapter canonical value was refused", 70);
}

export function providerAdapterRegistryBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function providerAdapterRegistryDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    fail(`${label} was malformed`, 70);
  }
  if (canonical(Object.keys(value).sort()) !== canonical([...keys].sort())) {
    fail(`${label} fields were refused`, 70);
  }
}

function lowerHex(value, length) {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/.test(value);
}

function sameCanonical(left, right) {
  const leftBytes = providerAdapterRegistryBytes(left);
  const rightBytes = providerAdapterRegistryBytes(right);
  return (
    leftBytes.length === rightBytes.length && timingSafeEqual(leftBytes, rightBytes)
  );
}

export const PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES = deepFreeze({
  execution_authorized: false,
  finalization_eligible: false,
  lifecycle_exposure_authorized: false,
  recovery_authorized: false,
  state_planning_authorized: false,
});

export const PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES = deepFreeze({
  execution_authorized: false,
  finalization_eligible: false,
  lifecycle_exposure_authorized: false,
  recovery_authorized: false,
  state_planning_authorized: true,
});

export const COLIMA_LIVE_CREATE_OPERATION_CONTRACT = deepFreeze({
  action: "provider-create",
  capabilities: PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
  evidence_schema: COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA,
  operation_kind: COLIMA_LIVE_CREATE_OPERATION_KIND,
  preparation_observation_schema: COLIMA_LIVE_OBSERVATION_SCHEMA,
  provider_class: COLIMA_LIVE_PROVIDER_CLASS,
  requirements_sha256: COLIMA_LIVE_REQUIREMENTS_SHA256,
  schema: COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SCHEMA,
  state_integration: "mutation-journal-v3-plan-only",
});

export const COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256 =
  providerAdapterRegistryDigest(
    providerAdapterRegistryBytes(COLIMA_LIVE_CREATE_OPERATION_CONTRACT),
  );

export const COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT = deepFreeze({
  action: "provider-cleanup",
  capabilities: PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES,
  create_operation_contract_sha256:
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  evidence_schema: COLIMA_LIVE_CLEANUP_EVIDENCE_SCHEMA,
  operation_kind: COLIMA_LIVE_CLEANUP_OPERATION_KIND,
  provider_class: COLIMA_LIVE_PROVIDER_CLASS,
  requirements_sha256: COLIMA_LIVE_REQUIREMENTS_SHA256,
  schema: COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SCHEMA,
  state_integration: "not-authorized",
});

export const COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SHA256 =
  providerAdapterRegistryDigest(
    providerAdapterRegistryBytes(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT),
  );

function validateCapabilities(value, expected) {
  exactKeys(
    value,
    [
      "execution_authorized",
      "finalization_eligible",
      "lifecycle_exposure_authorized",
      "recovery_authorized",
      "state_planning_authorized",
    ],
    "provider adapter capabilities",
  );
  if (!sameCanonical(value, expected)) {
    fail("provider adapter capabilities were refused", 69);
  }
}

export function validateColimaLiveCreateOperationContract(value) {
  exactKeys(
    value,
    [
      "action",
      "capabilities",
      "evidence_schema",
      "operation_kind",
      "preparation_observation_schema",
      "provider_class",
      "requirements_sha256",
      "schema",
      "state_integration",
    ],
    "Colima live create operation contract",
  );
  validateCapabilities(value.capabilities, PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES);
  if (!sameCanonical(value, COLIMA_LIVE_CREATE_OPERATION_CONTRACT)) {
    fail("Colima live create operation contract was refused");
  }
  return value;
}

export function validateColimaLiveCleanupOperationContract(value) {
  exactKeys(
    value,
    [
      "action",
      "capabilities",
      "create_operation_contract_sha256",
      "evidence_schema",
      "operation_kind",
      "provider_class",
      "requirements_sha256",
      "schema",
      "state_integration",
    ],
    "Colima live cleanup operation contract",
  );
  validateCapabilities(value.capabilities, PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES);
  if (!sameCanonical(value, COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT)) {
    fail("Colima live cleanup operation contract was refused");
  }
  return value;
}

function validateTuple(value) {
  exactKeys(value, TUPLE_FIELDS, "provider adapter tuple");
  if (
    !["provider-create", "provider-cleanup"].includes(value.action) ||
    typeof value.operation_kind !== "string" ||
    !/^[a-z0-9-]{1,96}$/.test(value.operation_kind) ||
    !lowerHex(value.operation_contract_sha256, 64) ||
    typeof value.provider_class !== "string" ||
    !/^[a-z0-9-]{1,96}$/.test(value.provider_class)
  ) {
    fail("provider adapter tuple was refused", 69);
  }
}

function keyValue(value) {
  return {
    action: value.action,
    operation_contract_sha256: value.operation_contract_sha256,
    operation_kind: value.operation_kind,
    provider_class: value.provider_class,
    schema: PROVIDER_ADAPTER_KEY_SCHEMA,
  };
}

export function providerAdapterRegistryKey(value) {
  validateTuple(value);
  return providerAdapterRegistryDigest(providerAdapterRegistryBytes(keyValue(value)));
}

function entryFor(contract, operationContractSha256) {
  const tuple = {
    action: contract.action,
    operation_contract_sha256: operationContractSha256,
    operation_kind: contract.operation_kind,
    provider_class: contract.provider_class,
  };
  return {
    action: tuple.action,
    adapter_id: ADAPTER_ID,
    capabilities: contract.capabilities,
    evidence_schema: contract.evidence_schema,
    key_sha256: providerAdapterRegistryKey(tuple),
    operation_contract_sha256: tuple.operation_contract_sha256,
    operation_kind: tuple.operation_kind,
    provider_class: tuple.provider_class,
    requirements_sha256: contract.requirements_sha256,
    state_integration: contract.state_integration,
  };
}

export const PROVIDER_ADAPTER_REGISTRY = deepFreeze({
  entries: [
    entryFor(
      COLIMA_LIVE_CREATE_OPERATION_CONTRACT,
      COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
    ),
    entryFor(
      COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT,
      COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SHA256,
    ),
  ],
  schema: PROVIDER_ADAPTER_REGISTRY_SCHEMA,
  selection_policy: "exact-action-kind-contract-class-tuple-v1",
});

export const PROVIDER_ADAPTER_REGISTRY_SHA256 = providerAdapterRegistryDigest(
  providerAdapterRegistryBytes(PROVIDER_ADAPTER_REGISTRY),
);

function expectedContract(entry) {
  if (entry.action === "provider-create") {
    return {
      contract: COLIMA_LIVE_CREATE_OPERATION_CONTRACT,
      sha256: COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
    };
  }
  if (entry.action === "provider-cleanup") {
    return {
      contract: COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT,
      sha256: COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SHA256,
    };
  }
  fail("provider adapter registry action was refused");
}

function validateEntry(value) {
  exactKeys(
    value,
    [
      "action",
      "adapter_id",
      "capabilities",
      "evidence_schema",
      "key_sha256",
      "operation_contract_sha256",
      "operation_kind",
      "provider_class",
      "requirements_sha256",
      "state_integration",
    ],
    "provider adapter registry entry",
  );
  const expected = expectedContract(value);
  validateCapabilities(value.capabilities, expected.contract.capabilities);
  const tuple = {
    action: value.action,
    operation_contract_sha256: value.operation_contract_sha256,
    operation_kind: value.operation_kind,
    provider_class: value.provider_class,
  };
  if (
    value.adapter_id !== ADAPTER_ID ||
    value.evidence_schema !== expected.contract.evidence_schema ||
    value.key_sha256 !== providerAdapterRegistryKey(tuple) ||
    value.operation_contract_sha256 !== expected.sha256 ||
    value.operation_kind !== expected.contract.operation_kind ||
    value.provider_class !== COLIMA_LIVE_PROVIDER_CLASS ||
    value.requirements_sha256 !== COLIMA_LIVE_REQUIREMENTS_SHA256 ||
    value.state_integration !== expected.contract.state_integration
  ) {
    fail("provider adapter registry entry was refused");
  }
  return value;
}

export function validateProviderAdapterRegistry(value) {
  validateColimaLiveCreateOperationContract(COLIMA_LIVE_CREATE_OPERATION_CONTRACT);
  validateColimaLiveCleanupOperationContract(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT);
  exactKeys(
    value,
    ["entries", "schema", "selection_policy"],
    "provider adapter registry",
  );
  if (
    value.schema !== PROVIDER_ADAPTER_REGISTRY_SCHEMA ||
    value.selection_policy !== "exact-action-kind-contract-class-tuple-v1" ||
    !Array.isArray(value.entries) ||
    value.entries.length !== 2
  ) {
    fail("provider adapter registry was refused");
  }
  const keys = new Set();
  for (const entry of value.entries) {
    validateEntry(entry);
    if (keys.has(entry.key_sha256)) fail("provider adapter registry key was duplicated");
    keys.add(entry.key_sha256);
  }
  if (!sameCanonical(value, PROVIDER_ADAPTER_REGISTRY)) {
    fail("provider adapter registry order or content was refused");
  }
  return value;
}

export function resolveProviderAdapter(value) {
  validateTuple(value);
  validateProviderAdapterRegistry(PROVIDER_ADAPTER_REGISTRY);
  const keySha256 = providerAdapterRegistryKey(value);
  const entry = PROVIDER_ADAPTER_REGISTRY.entries.find(
    (candidate) =>
      candidate.key_sha256 === keySha256 &&
      candidate.action === value.action &&
      candidate.operation_kind === value.operation_kind &&
      candidate.operation_contract_sha256 === value.operation_contract_sha256 &&
      candidate.provider_class === value.provider_class,
  );
  if (entry === undefined) fail("provider adapter tuple was refused", 69);
  return deepFreeze({
    action: entry.action,
    adapter_id: entry.adapter_id,
    capabilities: { ...entry.capabilities },
    evidence_schema: entry.evidence_schema,
    key_sha256: entry.key_sha256,
    operation_contract_sha256: entry.operation_contract_sha256,
    operation_kind: entry.operation_kind,
    provider_class: entry.provider_class,
    registry_sha256: PROVIDER_ADAPTER_REGISTRY_SHA256,
    requirements_sha256: entry.requirements_sha256,
    schema: PROVIDER_ADAPTER_RESOLUTION_SCHEMA,
    state_integration: entry.state_integration,
  });
}

export function authorizeProviderAdapter(value) {
  resolveProviderAdapter(value);
  fail("provider adapter execution remains disabled", 69);
}

export function authorizeProviderAdapterPlanning(value) {
  const resolution = resolveProviderAdapter(value);
  if (
    resolution.action !== "provider-create" ||
    resolution.state_integration !== "mutation-journal-v3-plan-only" ||
    !sameCanonical(
      resolution.capabilities,
      PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
    )
  ) {
    fail("provider adapter state planning was refused", 69);
  }
  return resolution;
}

validateProviderAdapterRegistry(PROVIDER_ADAPTER_REGISTRY);
