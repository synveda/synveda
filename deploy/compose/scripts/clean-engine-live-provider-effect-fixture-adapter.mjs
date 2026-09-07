#!/usr/bin/env node
import { createHash } from "node:crypto";
import { isProxy } from "node:util/types";

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-contract.v1";
export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-result.v1";

const FIXTURE_OPERATION_KIND = "colima-live-fixture-provider-effect-v1";
const FIXTURE_OPERATION_CONTRACT_SHA256 =
  "2926b334f9f63a665fc3648e32e3438627bff04a9dd90d1fed9b71e3d23aeae8";
const ZERO_SHA256 = "0".repeat(64);
const INPUT_FIELDS = Object.freeze([
  "adapter_contract_sha256",
  "attempt_event_sha256",
  "driver_contract_sha256",
  "fixture_id",
  "operation_contract_sha256",
  "operation_kind",
  "outer_launch_edge_sha256",
  "planned_start_attempt_sha256",
]);
const RESULT_FIELDS = Object.freeze([
  "attempt_event_sha256",
  "child_handle_identity_sha256",
  "delivery_disposition",
  "driver_contract_sha256",
  "effect_possible",
  "observation_sha256",
  "outer_launch_edge_sha256",
  "safe_error_code",
  "schema",
]);

export class LiveProviderEffectFixtureAdapterFailure extends Error {
  constructor(message, exitStatus = 69) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 69) {
  throw new LiveProviderEffectFixtureAdapterFailure(message, exitStatus);
}

function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    !Object.is(value, -0)
  ) {
    return String(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
    .join(",")}}`;
}

function bytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

function digest(value) {
  return createHash("sha256").update(bytes(value)).digest("hex");
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT = deepFreeze({
  adapter: "state-owned-conclusive-not-created-v1",
  authority: "none-fixture-process-free-result-only",
  capabilities: {
    child_handle_creation: false,
    filesystem_access: false,
    network_access: false,
    process_invocation: false,
    provider_effect_possible: false,
    runtime_publication: false,
  },
  evidence_class: "fixture-only",
  fixed_delivery_disposition: "conclusive-not-created",
  idempotency: "same-closed-input-same-result",
  input_fields: [...INPUT_FIELDS],
  operation_contract_sha256: FIXTURE_OPERATION_CONTRACT_SHA256,
  operation_kind: FIXTURE_OPERATION_KIND,
  result_fields: [...RESULT_FIELDS],
  result_schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA,
  schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SCHEMA,
});

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SHA256 =
  digest(COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT);

function closedRecord(value, fields, label) {
  if (
    isProxy(value) ||
    value === null ||
    Array.isArray(value) ||
    typeof value !== "object" ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    fail(`${label} was refused`, 70);
  }
  const keys = Reflect.ownKeys(value);
  if (
    keys.some((key) => typeof key !== "string") ||
    JSON.stringify([...keys].sort()) !== JSON.stringify([...fields].sort())
  ) {
    fail(`${label} fields were refused`, 70);
  }
  for (const key of keys) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (
      descriptor === undefined ||
      !("value" in descriptor) ||
      descriptor.enumerable !== true
    ) {
      fail(`${label} contained a hidden or accessor property`, 70);
    }
  }
  return value;
}

function nonzeroSha256(value) {
  return (
    typeof value === "string" &&
    /^[0-9a-f]{64}$/u.test(value) &&
    value !== ZERO_SHA256
  );
}

function validateInput(value) {
  closedRecord(value, INPUT_FIELDS, "fixture provider effect adapter input");
  const digests = [
    value.adapter_contract_sha256,
    value.attempt_event_sha256,
    value.driver_contract_sha256,
    value.operation_contract_sha256,
    value.outer_launch_edge_sha256,
    value.planned_start_attempt_sha256,
  ];
  if (
    value.adapter_contract_sha256 !==
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SHA256 ||
    value.operation_kind !== FIXTURE_OPERATION_KIND ||
    value.operation_contract_sha256 !== FIXTURE_OPERATION_CONTRACT_SHA256 ||
    typeof value.fixture_id !== "string" ||
    !/^[0-9a-f]{32}$/u.test(value.fixture_id) ||
    digests.some((entry) => !nonzeroSha256(entry)) ||
    new Set(digests).size !== digests.length
  ) {
    fail("fixture provider effect adapter identity was refused");
  }
  return value;
}

function resultFor(input) {
  const fixedResult = {
    attempt_event_sha256: input.attempt_event_sha256,
    child_handle_identity_sha256: ZERO_SHA256,
    delivery_disposition: "conclusive-not-created",
    driver_contract_sha256: input.driver_contract_sha256,
    effect_possible: false,
    outer_launch_edge_sha256: input.outer_launch_edge_sha256,
    safe_error_code: "not-created",
  };
  return {
    ...fixedResult,
    observation_sha256: digest({
      adapter_contract_sha256: input.adapter_contract_sha256,
      domain:
        "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-result.v1",
      fixture_id: input.fixture_id,
      operation_contract_sha256: input.operation_contract_sha256,
      operation_kind: input.operation_kind,
      planned_start_attempt_sha256: input.planned_start_attempt_sha256,
      result: fixedResult,
      schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA,
    }),
    schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA,
  };
}

export function validateColimaLiveProviderEffectFixtureAdapterResult(
  value,
  input,
) {
  closedRecord(value, RESULT_FIELDS, "fixture provider effect adapter result");
  validateInput(input);
  const expected = resultFor(input);
  if (RESULT_FIELDS.some((field) => value[field] !== expected[field])) {
    fail("fixture provider effect adapter result changed", 70);
  }
  return value;
}

export function executeColimaLiveProviderEffectFixtureConclusiveAdapter(input) {
  validateInput(input);
  const value = resultFor(input);
  validateColimaLiveProviderEffectFixtureAdapterResult(value, input);
  return deepFreeze(value);
}
