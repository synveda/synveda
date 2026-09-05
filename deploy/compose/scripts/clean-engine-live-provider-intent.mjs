#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  LiveProviderPlanFailure,
  liveProviderPlanBytes,
  liveProviderPlanDigest,
  validateColimaLiveProviderOperationPlan,
} from "./clean-engine-live-provider-plan.mjs";

export const COLIMA_LIVE_PLAN_COMPLETION_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-plan-completion-projection.v1";
export const COLIMA_LIVE_EFFECT_INTENT_CANDIDATE_SCHEMA =
  "synveda.clean-engine.colima-live-effect-intent-candidate.v1";
export const COLIMA_LIVE_EMPTY_PRE_EFFECT_PREFIX_SCHEMA =
  "synveda.clean-engine.colima-live-empty-pre-effect-prefix.v1";

const PROJECTION_FIELDS = Object.freeze([
  "operation_plan_sha256",
  "plan_close_sha256",
  "plan_slot_sha256",
  "preparation_observation_sha256",
  "schema",
]);
const INTENT_FIELDS = Object.freeze([
  "completed_plan_projection_sha256",
  "effect_authorization",
  "effect_name",
  "preparation_observation_sha256",
  "schema",
]);
const PREFIX_FIELDS = Object.freeze([
  "entry_count",
  "intent_candidate_sha256",
  "schema",
]);
const ZERO_SHA256 = "0".repeat(64);

// These helpers close canonical data shapes and their internal digest bindings.
// They do not authenticate state or observation provenance: serialized values
// can be manufactured or replayed. Only a direct return from the state owner
// reflects the state that owner read, and the next admission boundary must
// obtain that value internally while independently reopening the observation.

export class LiveProviderIntentFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderIntentFailure(message, exitStatus);
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
  fail("live provider intent canonical value was refused", 70);
}

export function liveProviderIntentBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderIntentDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function valueDigest(value) {
  return liveProviderIntentDigest(liveProviderIntentBytes(value));
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

function nonzeroSha256(value) {
  return (
    typeof value === "string" &&
    value.length === 64 &&
    /^[0-9a-f]+$/u.test(value) &&
    value !== ZERO_SHA256
  );
}

function validateOperationPlan(value) {
  try {
    return validateColimaLiveProviderOperationPlan(value);
  } catch (error) {
    if (error instanceof LiveProviderPlanFailure) {
      fail("live provider intent operation plan was refused", error.exitStatus);
    }
    throw error;
  }
}

export function validateColimaLivePlanCompletionProjectionStructure(value) {
  exactKeys(value, PROJECTION_FIELDS, "live provider plan completion projection");
  if (
    value.schema !== COLIMA_LIVE_PLAN_COMPLETION_PROJECTION_SCHEMA ||
    !nonzeroSha256(value.plan_slot_sha256) ||
    !nonzeroSha256(value.plan_close_sha256) ||
    !nonzeroSha256(value.operation_plan_sha256) ||
    !nonzeroSha256(value.preparation_observation_sha256)
  ) {
    fail("live provider plan completion projection was refused", 69);
  }
  return value;
}

export function buildColimaLivePlanCompletionProjectionStructure(value) {
  exactKeys(
    value,
    [
      "operationPlanSha256",
      "planCloseSha256",
      "planSlotSha256",
      "preparationObservationSha256",
    ],
    "live provider plan completion projection input",
  );
  const projection = {
    operation_plan_sha256: value.operationPlanSha256,
    plan_close_sha256: value.planCloseSha256,
    plan_slot_sha256: value.planSlotSha256,
    preparation_observation_sha256: value.preparationObservationSha256,
    schema: COLIMA_LIVE_PLAN_COMPLETION_PROJECTION_SCHEMA,
  };
  validateColimaLivePlanCompletionProjectionStructure(projection);
  return deepFreeze(projection);
}

export function validateColimaLiveEffectIntentCandidateStructure(
  value,
  completionProjection,
  operationPlan,
) {
  exactKeys(value, INTENT_FIELDS, "live provider effect intent candidate");
  validateColimaLivePlanCompletionProjectionStructure(completionProjection);
  validateOperationPlan(operationPlan);
  if (
    value.schema !== COLIMA_LIVE_EFFECT_INTENT_CANDIDATE_SCHEMA ||
    value.effect_name !== "provider-create" ||
    value.effect_authorization !== "requested-not-authorized" ||
    value.completed_plan_projection_sha256 !== valueDigest(completionProjection) ||
    value.preparation_observation_sha256 !==
      completionProjection.preparation_observation_sha256 ||
    completionProjection.operation_plan_sha256 !==
      liveProviderPlanDigest(liveProviderPlanBytes(operationPlan)) ||
    completionProjection.preparation_observation_sha256 !==
      operationPlan.preparation_observation_sha256
  ) {
    fail("live provider effect intent candidate was refused", 69);
  }
  return value;
}

export function buildColimaLiveEffectIntentCandidateStructure(value) {
  exactKeys(
    value,
    ["completionProjection", "operationPlan"],
    "live provider effect intent structure input",
  );
  validateColimaLivePlanCompletionProjectionStructure(value.completionProjection);
  validateOperationPlan(value.operationPlan);
  const intent = {
    completed_plan_projection_sha256: valueDigest(value.completionProjection),
    effect_authorization: "requested-not-authorized",
    effect_name: "provider-create",
    preparation_observation_sha256:
      value.completionProjection.preparation_observation_sha256,
    schema: COLIMA_LIVE_EFFECT_INTENT_CANDIDATE_SCHEMA,
  };
  validateColimaLiveEffectIntentCandidateStructure(
    intent,
    value.completionProjection,
    value.operationPlan,
  );
  return deepFreeze(intent);
}

export function validateColimaLiveEmptyPreEffectPrefixStructure(
  value,
  intent,
  completionProjection,
  operationPlan,
) {
  exactKeys(value, PREFIX_FIELDS, "live provider empty pre-effect prefix");
  validateColimaLiveEffectIntentCandidateStructure(
    intent,
    completionProjection,
    operationPlan,
  );
  if (
    value.schema !== COLIMA_LIVE_EMPTY_PRE_EFFECT_PREFIX_SCHEMA ||
    value.intent_candidate_sha256 !== valueDigest(intent) ||
    value.entry_count !== 0
  ) {
    fail("live provider empty pre-effect prefix was refused", 69);
  }
  return value;
}

export function buildColimaLiveEmptyPreEffectPrefixStructure(value) {
  exactKeys(
    value,
    ["completionProjection", "intent", "operationPlan"],
    "live provider empty pre-effect prefix build input",
  );
  validateColimaLiveEffectIntentCandidateStructure(
    value.intent,
    value.completionProjection,
    value.operationPlan,
  );
  const prefix = {
    entry_count: 0,
    intent_candidate_sha256: valueDigest(value.intent),
    schema: COLIMA_LIVE_EMPTY_PRE_EFFECT_PREFIX_SCHEMA,
  };
  validateColimaLiveEmptyPreEffectPrefixStructure(
    prefix,
    value.intent,
    value.completionProjection,
    value.operationPlan,
  );
  return deepFreeze(prefix);
}
