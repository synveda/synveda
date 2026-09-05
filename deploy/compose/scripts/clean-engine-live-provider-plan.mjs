#!/usr/bin/env node
import { createHash, timingSafeEqual } from "node:crypto";
import {
  COLIMA_LIVE_OBSERVATION_SCHEMA,
  ColimaLiveContractFailure,
  colimaLiveBytes,
  colimaLiveDigest,
  revalidateColimaLiveObservation,
} from "./clean-engine-colima-live-contract.mjs";
import {
  COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA,
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_CREATE_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_CLASS,
  PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
  PROVIDER_ADAPTER_REGISTRY_SHA256,
  PROVIDER_ADAPTER_RESOLUTION_SCHEMA,
  ProviderAdapterRegistryFailure,
  authorizeProviderAdapterPlanning,
} from "./clean-engine-provider-adapter-registry.mjs";

export const COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-provider-operation-plan.v1";
export const COLIMA_LIVE_PROVIDER_PLAN_STATE_INTEGRATION =
  "mutation-journal-v3-plan-only";

const ZERO_SHA256 = "0".repeat(64);

const STATE_BINDING_FIELDS = Object.freeze([
  "candidate_sha256",
  "fixture_id",
  "source_head_sha256",
  "source_sequence",
]);
const TUPLE_FIELDS = Object.freeze([
  "action",
  "operation_contract_sha256",
  "operation_kind",
  "provider_class",
]);

export class LiveProviderPlanFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderPlanFailure(message, exitStatus);
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
  fail("live provider plan canonical value was refused", 70);
}

export function liveProviderPlanBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderPlanDigest(value) {
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
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/u.test(value);
}

function sameCanonical(left, right) {
  const leftBytes = liveProviderPlanBytes(left);
  const rightBytes = liveProviderPlanBytes(right);
  return (
    leftBytes.length === rightBytes.length && timingSafeEqual(leftBytes, rightBytes)
  );
}

function validateStateBinding(value) {
  exactKeys(value, STATE_BINDING_FIELDS, "live provider state binding");
  if (
    !lowerHex(value.candidate_sha256, 64) ||
    value.candidate_sha256 === ZERO_SHA256 ||
    !lowerHex(value.fixture_id, 32) ||
    !lowerHex(value.source_head_sha256, 64) ||
    value.source_head_sha256 === ZERO_SHA256 ||
    value.source_sequence !== 0
  ) {
    fail("live provider state binding was refused", 69);
  }
  return value;
}

function resolveCreateTuple(value) {
  exactKeys(value, TUPLE_FIELDS, "live provider adapter tuple");
  let resolution;
  try {
    resolution = authorizeProviderAdapterPlanning(value);
  } catch (error) {
    if (error instanceof ProviderAdapterRegistryFailure) {
      fail("live provider adapter tuple was refused", error.exitStatus);
    }
    throw error;
  }
  if (
    resolution.schema !== PROVIDER_ADAPTER_RESOLUTION_SCHEMA ||
    resolution.action !== "provider-create" ||
    resolution.operation_kind !== COLIMA_LIVE_CREATE_OPERATION_KIND ||
    resolution.operation_contract_sha256 !==
      COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256 ||
    resolution.provider_class !== COLIMA_LIVE_PROVIDER_CLASS ||
    resolution.evidence_schema !== COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA ||
    resolution.registry_sha256 !== PROVIDER_ADAPTER_REGISTRY_SHA256 ||
    resolution.state_integration !== COLIMA_LIVE_PROVIDER_PLAN_STATE_INTEGRATION ||
    !sameCanonical(
      resolution.capabilities,
      PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
    )
  ) {
    fail("live provider create resolution was refused", 69);
  }
  return resolution;
}

function operationPlanFromValidatedObservation({ observation, stateBinding, tuple }) {
  validateStateBinding(stateBinding);
  const resolution = resolveCreateTuple(tuple);
  if (
    observation.fixture_id !== stateBinding.fixture_id ||
    observation.schema !== COLIMA_LIVE_OBSERVATION_SCHEMA ||
    observation.provider_class !== resolution.provider_class ||
    observation.requirements_sha256 !== resolution.requirements_sha256 ||
    observation.provider_profile !== `synveda-cpr45-${stateBinding.fixture_id}`
  ) {
    fail("live provider observation state identity was refused", 69);
  }
  return deepFreeze({
    action: resolution.action,
    adapter_key_sha256: resolution.key_sha256,
    capabilities: { ...PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES },
    evidence_schema: resolution.evidence_schema,
    fixture_id: stateBinding.fixture_id,
    operation_contract_sha256: resolution.operation_contract_sha256,
    operation_kind: resolution.operation_kind,
    preparation_observation_schema: COLIMA_LIVE_OBSERVATION_SCHEMA,
    preparation_observation_sha256: colimaLiveDigest(colimaLiveBytes(observation)),
    provider_class: resolution.provider_class,
    provider_profile: observation.provider_profile,
    provider_resource: `synveda-cpr45-${stateBinding.fixture_id}`,
    registry_sha256: resolution.registry_sha256,
    requirements_sha256: resolution.requirements_sha256,
    schema: COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA,
    source_candidate_sha256: stateBinding.candidate_sha256,
    source_head_sha256: stateBinding.source_head_sha256,
    source_sequence: stateBinding.source_sequence,
    state_integration: COLIMA_LIVE_PROVIDER_PLAN_STATE_INTEGRATION,
  });
}

function validateBuildArguments(value) {
  exactKeys(
    value,
    [
      "observation",
      "observationInput",
      "stateBinding",
      "tuple",
    ],
    "live provider plan build input",
  );
  validateStateBinding(value.stateBinding);
  return value;
}

export function buildColimaLiveProviderOperationPlan(value) {
  validateBuildArguments(value);
  try {
    revalidateColimaLiveObservation(value.observation, value.observationInput);
  } catch (error) {
    if (error instanceof ColimaLiveContractFailure) {
      fail("live provider preparation observation was refused", error.exitStatus);
    }
    throw error;
  }
  const operationPlan = operationPlanFromValidatedObservation(value);
  validateColimaLiveProviderOperationPlan(operationPlan);
  return operationPlan;
}

export function validateColimaLiveProviderOperationPlan(value) {
  exactKeys(
    value,
    [
      "action",
      "adapter_key_sha256",
      "capabilities",
      "evidence_schema",
      "fixture_id",
      "operation_contract_sha256",
      "operation_kind",
      "preparation_observation_schema",
      "preparation_observation_sha256",
      "provider_class",
      "provider_profile",
      "provider_resource",
      "registry_sha256",
      "requirements_sha256",
      "schema",
      "source_candidate_sha256",
      "source_head_sha256",
      "source_sequence",
      "state_integration",
    ],
    "live provider operation plan",
  );
  const resolution = resolveCreateTuple({
    action: value.action,
    operation_contract_sha256: value.operation_contract_sha256,
    operation_kind: value.operation_kind,
    provider_class: value.provider_class,
  });
  validateStateBinding({
    candidate_sha256: value.source_candidate_sha256,
    fixture_id: value.fixture_id,
    source_head_sha256: value.source_head_sha256,
    source_sequence: value.source_sequence,
  });
  if (
    value.schema !== COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA ||
    value.adapter_key_sha256 !== resolution.key_sha256 ||
    value.adapter_key_sha256 === ZERO_SHA256 ||
    !sameCanonical(value.capabilities, PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES) ||
    value.evidence_schema !== resolution.evidence_schema ||
    value.preparation_observation_schema !== COLIMA_LIVE_OBSERVATION_SCHEMA ||
    !lowerHex(value.preparation_observation_sha256, 64) ||
    value.preparation_observation_sha256 === ZERO_SHA256 ||
    value.provider_profile !== `synveda-cpr45-${value.fixture_id}` ||
    value.provider_resource !== `synveda-cpr45-${value.fixture_id}` ||
    value.registry_sha256 !== resolution.registry_sha256 ||
    value.registry_sha256 === ZERO_SHA256 ||
    value.requirements_sha256 !== resolution.requirements_sha256 ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_PLAN_STATE_INTEGRATION
  ) {
    fail("live provider operation plan was refused", 69);
  }
  return value;
}
