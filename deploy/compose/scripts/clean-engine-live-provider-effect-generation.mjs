#!/usr/bin/env node
import { createHash } from "node:crypto";
import { COLIMA_LIVE_PROVIDER_RESERVATION_NAME } from "./clean-engine-colima-live-schemas.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  LiveProviderProcessStartFailure,
  liveProviderProcessStartBytes,
  liveProviderProcessStartDigest,
  validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
} from "./clean-engine-live-provider-process-start.mjs";

export const COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-provider-effect-generation-prerequisite-projection.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-effect-generation-prerequisite-projection.v1";

const ZERO_SHA256 = "0".repeat(64);
const PROJECTION_FIELDS = Object.freeze([
  "admission_schema",
  "admission_sha256",
  "authorities",
  "authority",
  "branch",
  "completed_no_spawn_predecessor",
  "completed_start_decision_projection_sha256",
  "evidence_class",
  "fixed_marker_name",
  "future_atomic_generation",
  "future_effect_witness",
  "future_execution_prerequisites",
  "predecessor",
  "process_start_effect_candidate_sha256",
  "schema",
  "state_integration",
]);
const AUTHORITY_FIELDS = Object.freeze([
  "cleanup_authorized",
  "effect_execution_authorized",
  "effect_witness_publication_authorized",
  "environment_publication_authorized",
  "evidence_publication_authorized",
  "finalization_authorized",
  "lifecycle_exposed",
  "marker_link_authorized",
  "marker_retirement_authorized",
  "process_group_ownership_authorized",
  "process_signal_authorized",
  "process_spawn_authorized",
  "process_start_authorized",
  "provider_adapter_execution_authorized",
  "provider_artifact_publication_authorized",
  "provider_recovery_authorized",
  "provider_root_mutation_authorized",
  "receipt_publication_authorized",
  "runtime_publication_authorized",
  "start_attempt_publication_authorized",
  "start_authority_publication_authorized",
  "state_effect_slot_publication_authorized",
  "state_integration_authorized",
]);
const FUTURE_ATOMIC_GENERATION_FIELDS = Object.freeze([
  "mutation_close_schema",
  "mutation_recovery_root_schema",
  "mutation_recovery_schema",
  "mutation_slot_schema",
  "receipt_schema",
  "required",
]);
const FUTURE_EFFECT_WITNESS_FIELDS = Object.freeze([
  "adr_0103_witness_reuse_forbidden",
  "distinct_new_inode_required",
  "marker_continuity_boundary",
  "mutation_slot_inode_reuse_forbidden",
  "one_to_one_effect_slot_binding_required",
  "publication_authorized",
]);
const FUTURE_EXECUTION_PREREQUISITE_FIELDS = Object.freeze([
  "authenticated_endpoint_set_required",
  "causal_role_contract",
  "complete_bounded_causal_graph_required",
  "complete_recursive_inventory_required",
  "durable_quiescence_fence_required",
  "exact_invocation_binding_required",
  "exact_retirement_required",
  "planned_attempt_identity_required",
  "prerequisites_satisfied",
  "single_use_start_attempt_fence_required",
  "uncertain_start_is_blocking",
]);
const CAUSAL_ROLE_CONTRACT = Object.freeze([
  "outer",
  "hostagent",
  "usernet",
  "ssh-controlmaster",
]);

export class LiveProviderEffectGenerationFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderEffectGenerationFailure(message, exitStatus);
}

function closedDataEntries(value, label) {
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key !== "string")) {
    fail(`${label} contained a symbol property`, 70);
  }
  if (Array.isArray(value)) {
    const lengthDescriptor = Object.getOwnPropertyDescriptor(value, "length");
    if (
      Object.getPrototypeOf(value) !== Array.prototype ||
      lengthDescriptor === undefined ||
      !("value" in lengthDescriptor) ||
      !Number.isSafeInteger(lengthDescriptor.value) ||
      lengthDescriptor.value < 0 ||
      keys.length !== lengthDescriptor.value + 1
    ) {
      fail(`${label} was not a dense data array`, 70);
    }
    const entries = [];
    for (const key of keys) {
      if (key === "length") continue;
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      const index = Number(key);
      if (
        !/^(?:0|[1-9][0-9]*)$/u.test(key) ||
        !Number.isSafeInteger(index) ||
        index >= lengthDescriptor.value ||
        descriptor === undefined ||
        !("value" in descriptor) ||
        descriptor.enumerable !== true
      ) {
        fail(`${label} was not a dense data array`, 70);
      }
      entries.push([index, descriptor.value]);
    }
    entries.sort((left, right) => left[0] - right[0]);
    if (entries.some(([index], position) => index !== position)) {
      fail(`${label} was not a dense data array`, 70);
    }
    return Object.freeze({ entries, kind: "array" });
  }
  if (value === null || Object.getPrototypeOf(value) !== Object.prototype) {
    fail(`${label} did not have the plain data-object prototype`, 70);
  }
  const entries = [];
  for (const key of keys) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (
      descriptor === undefined ||
      !("value" in descriptor) ||
      descriptor.enumerable !== true
    ) {
      fail(`${label} contained a hidden or accessor property`, 70);
    }
    entries.push([key, descriptor.value]);
  }
  return Object.freeze({ entries, kind: "record" });
}

function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) {
    return String(value);
  }
  if (typeof value === "object") {
    const inspected = closedDataEntries(
      value,
      "live provider effect-generation canonical value",
    );
    if (inspected.kind === "array") {
      return `[${inspected.entries
        .map(([, child]) => canonical(child))
        .join(",")}]`;
    }
    return `{${inspected.entries
      .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
      .map(([key, child]) => `${JSON.stringify(key)}:${canonical(child)}`)
      .join(",")}}`;
  }
  fail("live provider effect-generation canonical value was refused", 70);
}

export function liveProviderEffectGenerationBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderEffectGenerationDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function processValueDigest(value) {
  return liveProviderProcessStartDigest(liveProviderProcessStartBytes(value));
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object") {
    for (const [, child] of closedDataEntries(
      value,
      "live provider effect-generation freeze value",
    ).entries) {
      deepFreeze(child);
    }
    if (!Object.isFrozen(value)) Object.freeze(value);
  }
  return value;
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    fail(`${label} was malformed`, 70);
  }
  const inspected = closedDataEntries(value, label);
  if (
    inspected.kind !== "record" ||
    canonical(inspected.entries.map(([key]) => key).sort()) !==
      canonical([...keys].sort())
  ) {
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

function variant(admission, source) {
  canonical({ admission, source });
  if (
    admission === null ||
    Array.isArray(admission) ||
    typeof admission !== "object" ||
    source === null ||
    Array.isArray(source) ||
    typeof source !== "object"
  ) {
    fail("live provider effect-generation source was malformed", 70);
  }
  let selected;
  if (
    admission.schema ===
      COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA ||
    admission.schema ===
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA
  ) {
    fail("effect-generation projection cannot be used as its own admission", 69);
  }
  if (
    admission.schema ===
      COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA &&
    source.fixtureOnly === false
  ) {
    selected = Object.freeze({
      admissionSchema:
        COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
      evidenceClass: "production-pinned",
      projectionSchema:
        COLIMA_LIVE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
    });
  } else if (
    admission.schema ===
      COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA &&
    source.fixtureOnly === true
  ) {
    selected = Object.freeze({
      admissionSchema:
        COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
      evidenceClass: "fixture-only",
      projectionSchema:
        COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_GENERATION_PREREQUISITE_PROJECTION_SCHEMA,
    });
  } else {
    fail("live provider effect-generation evidence class was refused", 69);
  }
  try {
    validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
      admission,
      source,
    );
  } catch (error) {
    if (error instanceof LiveProviderProcessStartFailure) {
      fail("live provider effect-generation source was refused", error.exitStatus);
    }
    throw error;
  }
  if (
    admission.process_start_effect_candidate === null ||
    admission.completed_start_decision_projection.evidence_class !==
      selected.evidenceClass ||
    admission.process_start_effect_candidate.evidence_class !==
      selected.evidenceClass
  ) {
    fail("live provider effect-generation prerequisite was refused", 69);
  }
  return selected;
}

function projectionFor(admission, source) {
  const selected = variant(admission, source);
  return {
    admission_schema: selected.admissionSchema,
    admission_sha256: processValueDigest(admission),
    authorities: Object.fromEntries(
      AUTHORITY_FIELDS.map((field) => [field, false]),
    ),
    authority: "none-prerequisite-projection-only",
    branch: "sibling-effect-prerequisite-only",
    completed_no_spawn_predecessor: "forbidden-terminal-sibling",
    completed_start_decision_projection_sha256: processValueDigest(
      admission.completed_start_decision_projection,
    ),
    evidence_class: selected.evidenceClass,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    future_atomic_generation: {
      mutation_close_schema: "synveda.clean-engine.mutation-close.v8",
      mutation_recovery_root_schema:
        "synveda.clean-engine.mutation-recovery-root.v6",
      mutation_recovery_schema: "synveda.clean-engine.mutation-recovery.v6",
      mutation_slot_schema: "synveda.clean-engine.mutation-slot.v7",
      receipt_schema: "synveda.clean-engine.receipt.v6",
      required: true,
    },
    future_effect_witness: {
      adr_0103_witness_reuse_forbidden: true,
      distinct_new_inode_required: true,
      marker_continuity_boundary:
        "before-start-authority-through-terminal-receipt",
      mutation_slot_inode_reuse_forbidden: true,
      one_to_one_effect_slot_binding_required: true,
      publication_authorized: false,
    },
    future_execution_prerequisites: {
      authenticated_endpoint_set_required: true,
      causal_role_contract: [...CAUSAL_ROLE_CONTRACT],
      complete_bounded_causal_graph_required: true,
      complete_recursive_inventory_required: true,
      durable_quiescence_fence_required: true,
      exact_invocation_binding_required: true,
      exact_retirement_required: true,
      planned_attempt_identity_required: true,
      prerequisites_satisfied: false,
      single_use_start_attempt_fence_required: true,
      uncertain_start_is_blocking: true,
    },
    predecessor: "completed-start-decision-only",
    process_start_effect_candidate_sha256: processValueDigest(
      admission.process_start_effect_candidate,
    ),
    schema: selected.projectionSchema,
    state_integration: "not-integrated",
  };
}

function validateProjectionShape(value) {
  canonical(value);
  exactKeys(
    value,
    PROJECTION_FIELDS,
    "live provider effect-generation prerequisite projection",
  );
  exactKeys(
    value.authorities,
    AUTHORITY_FIELDS,
    "live provider effect-generation authorities",
  );
  exactKeys(
    value.future_atomic_generation,
    FUTURE_ATOMIC_GENERATION_FIELDS,
    "live provider effect-generation future atomic generation",
  );
  exactKeys(
    value.future_effect_witness,
    FUTURE_EFFECT_WITNESS_FIELDS,
    "live provider effect-generation future witness",
  );
  exactKeys(
    value.future_execution_prerequisites,
    FUTURE_EXECUTION_PREREQUISITE_FIELDS,
    "live provider effect-generation future execution prerequisites",
  );
}

export function validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
  value,
  expected,
) {
  exactKeys(
    expected,
    ["admission", "source"],
    "live provider effect-generation projection expectation",
  );
  validateProjectionShape(value);
  const required = projectionFor(expected.admission, expected.source);
  for (const digestValue of [
    value.admission_sha256,
    value.completed_start_decision_projection_sha256,
    value.process_start_effect_candidate_sha256,
  ]) {
    if (!nonzeroSha256(digestValue)) {
      fail("live provider effect-generation digest was refused", 69);
    }
  }
  if (
    new Set([
      value.admission_sha256,
      value.completed_start_decision_projection_sha256,
      value.process_start_effect_candidate_sha256,
    ]).size !== 3 ||
    !liveProviderEffectGenerationBytes(value).equals(
      liveProviderEffectGenerationBytes(required),
    )
  ) {
    fail("live provider effect-generation prerequisite projection was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
  value,
) {
  canonical(value);
  exactKeys(
    value,
    ["admission", "source"],
    "live provider effect-generation projection input",
  );
  const projection = projectionFor(value.admission, value.source);
  validateColimaLiveProviderEffectGenerationPrerequisiteProjectionStructure(
    projection,
    value,
  );
  return deepFreeze(projection);
}
