#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
} from "./clean-engine-colima-live-schemas.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
  COLIMA_LIVE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
  LiveProviderStartDecisionFailure,
  liveProviderStartDecisionBytes,
  liveProviderStartDecisionDigest,
  validateColimaLiveProviderStartDecisionCompletion,
  validateColimaLiveProviderStartDecisionPublicationPlan,
} from "./clean-engine-live-provider-start-decision.mjs";

export const COLIMA_LIVE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-completed-provider-start-decision-projection.v1";
export const COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-completed-provider-start-decision-projection.v1";
export const COLIMA_LIVE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA =
  "synveda.clean-engine.colima-live-process-start-effect-candidate.v1";
export const COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-process-start-effect-candidate.v1";
export const COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA =
  "synveda.clean-engine.colima-live-process-start-effect-fresh-admission.v1";
export const COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-process-start-effect-fresh-admission.v1";

const ZERO_SHA256 = "0".repeat(64);
const PROJECTION_FIELDS = Object.freeze([
  "close_authority",
  "completed_intent_projection_sha256",
  "decision_candidate_sha256",
  "evidence_class",
  "provider_start_decision_close_sha256",
  "provider_start_decision_operation_contract_sha256",
  "provider_start_decision_operation_kind",
  "provider_start_decision_publication_plan_sha256",
  "provider_start_decision_slot_sha256",
  "schema",
]);
const ROOT_OBSERVATION_FIELDS = Object.freeze([
  "evidence_class",
  "planned_names",
  "preparation_observation_sha256",
  "requirements_sha256",
  "root_observations",
  "root_set_disposition",
  "schema",
]);
const ROOT_FIELDS = Object.freeze([
  "disposition",
  "namespace_identity_hmac_sha256",
  "observed_entry_set_hmac_sha256",
  "role",
]);
const CANDIDATE_FIELDS = Object.freeze([
  "cleanup_authorized",
  "completed_start_decision_projection_sha256",
  "cross_namespace_atomic_reservation",
  "durable_effect_slot_required",
  "effect_authorization",
  "effect_execution_authorized",
  "effect_name",
  "environment_publication_authorized",
  "evidence_class",
  "evidence_publication_authorized",
  "finalization_authorized",
  "future_effect_fresh_admission_required",
  "lifecycle_exposed",
  "process_group_ownership_authorized",
  "process_signal_authorized",
  "process_spawn_authorized",
  "process_start_authorized",
  "process_state",
  "provider_adapter_execution_authorized",
  "provider_artifact_publication_authorized",
  "provider_recovery_authorized",
  "provider_root_mutation_authorized",
  "receipt_publication_authorized",
  "root_observation_sha256",
  "runtime_publication_authorized",
  "schema",
  "state_effect_slot_publication_authorized",
  "supervisor_process_group_label",
]);
const ADMISSION_FIELDS = Object.freeze([
  "authority",
  "completed_start_decision_projection",
  "process_start_effect_candidate",
  "root_observation",
  "schema",
  "supervisor_process_group_label",
]);

// These structures are point-in-time, deny-only data. They cannot start,
// spawn or signal a process, mutate a root, publish evidence or grant authority.

export class LiveProviderProcessStartFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderProcessStartFailure(message, exitStatus);
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
  if (Object.getPrototypeOf(value) !== Object.prototype) {
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
      "live provider process start canonical value",
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
  fail("live provider process start canonical value was refused", 70);
}

export function liveProviderProcessStartBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderProcessStartDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function valueDigest(value) {
  return liveProviderProcessStartDigest(liveProviderProcessStartBytes(value));
}

function startDecisionValueDigest(value) {
  return liveProviderStartDecisionDigest(liveProviderStartDecisionBytes(value));
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object") {
    for (const [, child] of closedDataEntries(
      value,
      "live provider process start freeze value",
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

function variant(fixtureOnly) {
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider process start variant was refused", 70);
  }
  return fixtureOnly
    ? Object.freeze({
        admissionAuthority:
          "fixture-only-point-in-time-deny-only-not-effect-authority",
        admissionSchema:
          COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
        candidateSchema:
          COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
        decisionCompletionSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
        decisionContractSha256:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
        decisionOperationKind:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
        decisionPublicationPlanSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
        evidenceClass: "fixture-only",
        projectionSchema:
          COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
        rootObservationSchema:
          COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
      })
    : Object.freeze({
        admissionAuthority:
          "point-in-time-deny-only-not-effect-authority",
        admissionSchema:
          COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
        candidateSchema: COLIMA_LIVE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
        decisionCompletionSchema:
          COLIMA_LIVE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
        decisionContractSha256:
          COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
        decisionOperationKind:
          COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
        decisionPublicationPlanSchema:
          COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
        evidenceClass: "production-pinned",
        projectionSchema:
          COLIMA_LIVE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
        rootObservationSchema: COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
      });
}

function sourceContext(value) {
  canonical(value);
  exactKeys(
    value,
    [
      "closeAuthority",
      "fixtureOnly",
      "startDecisionCompletion",
      "startDecisionPublicationPlan",
      "startDecisionSource",
    ],
    "live provider process start source",
  );
  const selected = variant(value.fixtureOnly);
  if (
    value.closeAuthority !== "owner" ||
    value.startDecisionSource?.fixtureOnly !== value.fixtureOnly
  ) {
    fail("live provider process start source authority was refused", 69);
  }
  try {
    validateColimaLiveProviderStartDecisionPublicationPlan(
      value.startDecisionPublicationPlan,
      value.startDecisionSource,
    );
    validateColimaLiveProviderStartDecisionCompletion(
      value.startDecisionCompletion,
      {
        closeSha256:
          value.startDecisionCompletion?.decision_close_sha256,
        publicationPlan: value.startDecisionPublicationPlan,
        slotSha256: value.startDecisionCompletion?.decision_slot_sha256,
        source: value.startDecisionSource,
      },
    );
  } catch (error) {
    if (error instanceof LiveProviderStartDecisionFailure) {
      fail("live provider process start source decision was refused", error.exitStatus);
    }
    throw error;
  }
  if (
    value.startDecisionPublicationPlan.schema !==
      selected.decisionPublicationPlanSchema ||
    value.startDecisionCompletion.schema !== selected.decisionCompletionSchema ||
    value.startDecisionPublicationPlan.operation_kind !==
      selected.decisionOperationKind ||
    value.startDecisionPublicationPlan.operation_contract_sha256 !==
      selected.decisionContractSha256 ||
    value.startDecisionPublicationPlan.admission
      .process_start_decision_candidate.decision !==
      "requested-not-executed-not-authorized" ||
    value.startDecisionPublicationPlan.admission
      .process_start_decision_candidate.process_state !== "not-reached" ||
    value.startDecisionPublicationPlan.admission
      .process_start_decision_candidate.process_start_authorized !== false
  ) {
    fail("live provider process start source identity was refused", 69);
  }
  return Object.freeze({
    operationPlan:
      value.startDecisionSource.intentPublicationPlan.provider_operation_plan,
    selected,
  });
}

export function validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
  value,
  source,
) {
  exactKeys(
    value,
    PROJECTION_FIELDS,
    "live provider completed start decision projection",
  );
  const { selected } = sourceContext(source);
  const completion = source.startDecisionCompletion;
  if (
    value.schema !== selected.projectionSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.close_authority !== "owner" ||
    value.provider_start_decision_operation_kind !==
      selected.decisionOperationKind ||
    value.provider_start_decision_operation_contract_sha256 !==
      selected.decisionContractSha256 ||
    value.provider_start_decision_slot_sha256 !==
      completion.decision_slot_sha256 ||
    value.provider_start_decision_close_sha256 !==
      completion.decision_close_sha256 ||
    value.provider_start_decision_slot_sha256 ===
      value.provider_start_decision_close_sha256 ||
    value.provider_start_decision_publication_plan_sha256 !==
      completion.decision_publication_plan_sha256 ||
    value.provider_start_decision_publication_plan_sha256 !==
      startDecisionValueDigest(source.startDecisionPublicationPlan) ||
    value.decision_candidate_sha256 !== completion.decision_candidate_sha256 ||
    value.completed_intent_projection_sha256 !==
      completion.completed_intent_projection_sha256 ||
    !nonzeroSha256(value.provider_start_decision_slot_sha256) ||
    !nonzeroSha256(value.provider_start_decision_close_sha256) ||
    !nonzeroSha256(value.provider_start_decision_publication_plan_sha256) ||
    !nonzeroSha256(value.decision_candidate_sha256) ||
    !nonzeroSha256(value.completed_intent_projection_sha256)
  ) {
    fail("live provider completed start decision projection was refused", 69);
  }
  return value;
}

export function buildColimaLiveCompletedProviderStartDecisionProjectionStructure(
  source,
) {
  const { selected } = sourceContext(source);
  const completion = source.startDecisionCompletion;
  const projection = {
    close_authority: "owner",
    completed_intent_projection_sha256:
      completion.completed_intent_projection_sha256,
    decision_candidate_sha256: completion.decision_candidate_sha256,
    evidence_class: selected.evidenceClass,
    provider_start_decision_close_sha256: completion.decision_close_sha256,
    provider_start_decision_operation_contract_sha256:
      selected.decisionContractSha256,
    provider_start_decision_operation_kind: selected.decisionOperationKind,
    provider_start_decision_publication_plan_sha256:
      completion.decision_publication_plan_sha256,
    provider_start_decision_slot_sha256: completion.decision_slot_sha256,
    schema: selected.projectionSchema,
  };
  validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
    projection,
    source,
  );
  return deepFreeze(projection);
}

function validateRootObservation(value, source) {
  canonical(value);
  const { operationPlan, selected } = sourceContext(source);
  exactKeys(
    value,
    ROOT_OBSERVATION_FIELDS,
    "live provider process start root observation",
  );
  exactKeys(
    value.planned_names,
    ["lima_instance", "provider_profile"],
    "live provider process start planned names",
  );
  const roles = COLIMA_LIVE_MUTATION_SURFACE_ROLES;
  if (
    !Array.isArray(value.root_observations) ||
    value.root_observations.length !== roles.length
  ) {
    fail("live provider process start root observation was refused", 69);
  }
  for (const [index, root] of value.root_observations.entries()) {
    exactKeys(root, ROOT_FIELDS, "live provider process start root");
    if (
      root.role !== roles[index] ||
      !new Set(["foreign-collision", "observed-pristine"]).has(
        root.disposition,
      ) ||
      !nonzeroSha256(root.namespace_identity_hmac_sha256) ||
      !nonzeroSha256(root.observed_entry_set_hmac_sha256)
    ) {
      fail("live provider process start root was refused", 69);
    }
  }
  const disposition = value.root_observations.some(
    (root) => root.disposition === "foreign-collision",
  )
    ? "foreign-collision"
    : "observed-pristine";
  if (
    value.schema !== selected.rootObservationSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.root_set_disposition !== disposition ||
    !nonzeroSha256(value.requirements_sha256) ||
    value.requirements_sha256 !==
      source.startDecisionPublicationPlan.admission.root_observation
        .requirements_sha256 ||
    value.preparation_observation_sha256 !==
      operationPlan.preparation_observation_sha256 ||
    value.planned_names.provider_profile !== operationPlan.provider_profile ||
    value.planned_names.lima_instance !==
      `colima-${operationPlan.provider_profile}` ||
    operationPlan.provider_resource !== operationPlan.provider_profile
  ) {
    fail("live provider process start root observation binding was refused", 69);
  }
  return value;
}

function supervisorLabel(source) {
  const label =
    source.startDecisionPublicationPlan.admission
      .supervisor_process_group_label;
  if (
    !/^sv-c45-colima-pg-[0-9a-f]{32}-[0-9a-f]{12}$/u.test(label) ||
    Buffer.byteLength(label, "ascii") > 63
  ) {
    fail("live provider process start supervisor label was refused", 69);
  }
  return label;
}

export function validateColimaLiveProviderProcessStartEffectCandidateStructure(
  value,
  expected,
) {
  exactKeys(
    expected,
    ["completedStartDecisionProjection", "rootObservation", "source"],
    "live provider process start candidate expectation",
  );
  const { selected } = sourceContext(expected.source);
  validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
    expected.completedStartDecisionProjection,
    expected.source,
  );
  validateRootObservation(expected.rootObservation, expected.source);
  exactKeys(value, CANDIDATE_FIELDS, "live provider process start candidate");
  if (
    expected.rootObservation.root_set_disposition !== "observed-pristine" ||
    value.schema !== selected.candidateSchema ||
    value.completed_start_decision_projection_sha256 !==
      valueDigest(expected.completedStartDecisionProjection) ||
    value.root_observation_sha256 !== valueDigest(expected.rootObservation) ||
    value.effect_name !== "provider-process-start" ||
    value.effect_authorization !== "denied-no-durable-effect-contract" ||
    value.evidence_class !== selected.evidenceClass ||
    value.process_state !== "not-reached" ||
    value.process_start_authorized !== false ||
    value.process_spawn_authorized !== false ||
    value.process_signal_authorized !== false ||
    value.process_group_ownership_authorized !== false ||
    value.effect_execution_authorized !== false ||
    value.provider_adapter_execution_authorized !== false ||
    value.provider_root_mutation_authorized !== false ||
    value.provider_artifact_publication_authorized !== false ||
    value.evidence_publication_authorized !== false ||
    value.receipt_publication_authorized !== false ||
    value.environment_publication_authorized !== false ||
    value.runtime_publication_authorized !== false ||
    value.provider_recovery_authorized !== false ||
    value.cleanup_authorized !== false ||
    value.lifecycle_exposed !== false ||
    value.finalization_authorized !== false ||
    value.state_effect_slot_publication_authorized !== false ||
    value.cross_namespace_atomic_reservation !== false ||
    value.durable_effect_slot_required !== true ||
    value.future_effect_fresh_admission_required !== true ||
    value.supervisor_process_group_label !== supervisorLabel(expected.source)
  ) {
    fail("live provider process start candidate was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderProcessStartEffectCandidateStructure(
  value,
) {
  exactKeys(
    value,
    ["completedStartDecisionProjection", "rootObservation", "source"],
    "live provider process start candidate input",
  );
  const { selected } = sourceContext(value.source);
  validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
    value.completedStartDecisionProjection,
    value.source,
  );
  validateRootObservation(value.rootObservation, value.source);
  if (value.rootObservation.root_set_disposition !== "observed-pristine") {
    fail("live provider collision cannot become a process start candidate", 73);
  }
  const candidate = {
    cleanup_authorized: false,
    completed_start_decision_projection_sha256: valueDigest(
      value.completedStartDecisionProjection,
    ),
    cross_namespace_atomic_reservation: false,
    durable_effect_slot_required: true,
    effect_authorization: "denied-no-durable-effect-contract",
    effect_execution_authorized: false,
    effect_name: "provider-process-start",
    environment_publication_authorized: false,
    evidence_class: selected.evidenceClass,
    evidence_publication_authorized: false,
    finalization_authorized: false,
    future_effect_fresh_admission_required: true,
    lifecycle_exposed: false,
    process_group_ownership_authorized: false,
    process_signal_authorized: false,
    process_spawn_authorized: false,
    process_start_authorized: false,
    process_state: "not-reached",
    provider_adapter_execution_authorized: false,
    provider_artifact_publication_authorized: false,
    provider_recovery_authorized: false,
    provider_root_mutation_authorized: false,
    receipt_publication_authorized: false,
    root_observation_sha256: valueDigest(value.rootObservation),
    runtime_publication_authorized: false,
    schema: selected.candidateSchema,
    state_effect_slot_publication_authorized: false,
    supervisor_process_group_label: supervisorLabel(value.source),
  };
  validateColimaLiveProviderProcessStartEffectCandidateStructure(
    candidate,
    value,
  );
  return deepFreeze(candidate);
}

export function validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
  value,
  source,
) {
  const { selected } = sourceContext(source);
  exactKeys(value, ADMISSION_FIELDS, "live provider process start admission");
  validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
    value.completed_start_decision_projection,
    source,
  );
  validateRootObservation(value.root_observation, source);
  if (
    value.schema !== selected.admissionSchema ||
    value.authority !== selected.admissionAuthority ||
    value.supervisor_process_group_label !== supervisorLabel(source)
  ) {
    fail("live provider process start admission was refused", 69);
  }
  if (value.root_observation.root_set_disposition === "observed-pristine") {
    validateColimaLiveProviderProcessStartEffectCandidateStructure(
      value.process_start_effect_candidate,
      {
        completedStartDecisionProjection:
          value.completed_start_decision_projection,
        rootObservation: value.root_observation,
        source,
      },
    );
  } else if (value.process_start_effect_candidate !== null) {
    fail("live provider collision admitted a process start effect", 69);
  }
  return value;
}

export function buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
  value,
) {
  exactKeys(
    value,
    ["completedStartDecisionProjection", "rootObservation", "source"],
    "live provider process start admission input",
  );
  const { selected } = sourceContext(value.source);
  validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
    value.completedStartDecisionProjection,
    value.source,
  );
  validateRootObservation(value.rootObservation, value.source);
  const candidate =
    value.rootObservation.root_set_disposition === "observed-pristine"
      ? buildColimaLiveProviderProcessStartEffectCandidateStructure(value)
      : null;
  const admission = {
    authority: selected.admissionAuthority,
    completed_start_decision_projection:
      value.completedStartDecisionProjection,
    process_start_effect_candidate: candidate,
    root_observation: value.rootObservation,
    schema: selected.admissionSchema,
    supervisor_process_group_label: supervisorLabel(value.source),
  };
  validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
    admission,
    value.source,
  );
  return deepFreeze(admission);
}

export function authorizeColimaLiveProviderProcessStartEffect(argumentsValue) {
  exactKeys(
    argumentsValue,
    ["admission", "source"],
    "live provider process start authorization arguments",
  );
  sourceContext(argumentsValue.source);
  if (argumentsValue.source.fixtureOnly) {
    fail("fixture process start admission cannot authorize an effect", 69);
  }
  validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
    argumentsValue.admission,
    argumentsValue.source,
  );
  fail(
    "Colima live provider process start remains disabled pending durable effect authority",
    69,
  );
}
