#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
} from "./clean-engine-colima-live-schemas.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
  COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
  COLIMA_LIVE_PROVIDER_INTENT_STATE_INTEGRATION,
  LiveProviderIntentFailure,
  liveProviderIntentBytes,
  liveProviderIntentDigest,
  validateColimaLiveProviderIntentCompletion,
  validateColimaLiveProviderIntentPublicationPlan,
} from "./clean-engine-live-provider-intent.mjs";
import { COLIMA_LIVE_PROVIDER_CLASS } from "./clean-engine-provider-adapter-registry.mjs";

export const COLIMA_LIVE_COMPLETED_PROVIDER_INTENT_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-completed-provider-intent-projection.v1";
export const COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_INTENT_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-completed-provider-intent-projection.v1";
export const COLIMA_LIVE_PROCESS_START_DECISION_CANDIDATE_SCHEMA =
  "synveda.clean-engine.colima-live-process-start-decision-candidate.v1";
export const COLIMA_LIVE_FIXTURE_PROCESS_START_DECISION_CANDIDATE_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-process-start-decision-candidate.v1";
export const COLIMA_LIVE_PROCESS_START_FRESH_ADMISSION_SCHEMA =
  "synveda.clean-engine.colima-live-process-start-fresh-admission.v1";
export const COLIMA_LIVE_FIXTURE_PROCESS_START_FRESH_ADMISSION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-process-start-fresh-admission.v1";
export const COLIMA_LIVE_PROVIDER_START_DECISION_ACTION =
  "provider-start-decision";
export const COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND =
  "colima-live-provider-start-decision-publication-v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND =
  "colima-live-fixture-provider-start-decision-publication-v1";
export const COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-provider-start-decision-operation-contract.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-start-decision-operation-contract.v1";
export const COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-provider-start-decision-publication-plan.v2";
export const COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-start-decision-publication-plan.v2";
export const COLIMA_LIVE_PROVIDER_START_DECISION_COMPLETION_SCHEMA =
  "synveda.clean-engine.colima-live-provider-start-decision-completion.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-start-decision-completion.v1";
export const COLIMA_LIVE_PROVIDER_START_DECISION_STATE_INTEGRATION =
  "mutation-journal-v5-inert-start-decision-only";

const ZERO_SHA256 = "0".repeat(64);
const PROJECTION_FIELDS = Object.freeze([
  "close_authority",
  "completed_plan_projection_sha256",
  "evidence_class",
  "provider_intent_close_sha256",
  "provider_intent_operation_contract_sha256",
  "provider_intent_operation_kind",
  "provider_intent_publication_plan_sha256",
  "provider_intent_slot_sha256",
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
  "completed_intent_projection_sha256",
  "cross_namespace_atomic_reservation",
  "decision",
  "effect_name",
  "future_effect_fresh_admission_required",
  "process_start_authorized",
  "process_state",
  "root_observation_sha256",
  "schema",
  "supervisor_process_group_label",
]);
const ADMISSION_FIELDS = Object.freeze([
  "authority",
  "completed_intent_projection",
  "process_start_decision_candidate",
  "root_observation",
  "schema",
  "supervisor_process_group_label",
]);
const PUBLICATION_PLAN_FIELDS = Object.freeze([
  "admission",
  "admission_sha256",
  "completed_intent_projection_sha256",
  "decision_candidate_sha256",
  "evidence_class",
  "fixture_id",
  "operation_contract_sha256",
  "operation_kind",
  "publication_claim",
  "root_observation_sha256",
  "schema",
  "state_integration",
]);
const COMPLETION_FIELDS = Object.freeze([
  "authority",
  "completed_intent_projection_sha256",
  "decision_candidate_sha256",
  "decision_close_sha256",
  "decision_publication_plan_sha256",
  "decision_slot_sha256",
  "evidence_class",
  "schema",
]);

// These values close state-owner data shapes only. They carry no process-start
// or effect authority. The state owner must reconstruct their inputs directly
// and re-observe roots at every publication boundary.

export class LiveProviderStartDecisionFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderStartDecisionFailure(message, exitStatus);
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
  fail("live provider start decision canonical value was refused", 70);
}

export function liveProviderStartDecisionBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderStartDecisionDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function valueDigest(value) {
  return liveProviderStartDecisionDigest(liveProviderStartDecisionBytes(value));
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

function variant(fixtureOnly) {
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider start decision variant was refused", 70);
  }
  return fixtureOnly
    ? Object.freeze({
        admissionAuthority:
          "fixture-only-point-in-time-not-process-start-authority",
        admissionSchema: COLIMA_LIVE_FIXTURE_PROCESS_START_FRESH_ADMISSION_SCHEMA,
        candidateSchema:
          COLIMA_LIVE_FIXTURE_PROCESS_START_DECISION_CANDIDATE_SCHEMA,
        completionAuthority:
          "fixture-only-durable-inert-process-start-decision-not-effect-authority",
        completionSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
        contractSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SCHEMA,
        evidenceClass: "fixture-only",
        intentCompletionSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
        intentContractSha256:
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
        intentOperationKind: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
        intentPublicationPlanSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
        operationKind:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
        projectionSchema:
          COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_INTENT_PROJECTION_SCHEMA,
        publicationPlanSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
        rootObservationSchema:
          COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
      })
    : Object.freeze({
        admissionAuthority: "point-in-time-not-process-start-authority",
        admissionSchema: COLIMA_LIVE_PROCESS_START_FRESH_ADMISSION_SCHEMA,
        candidateSchema: COLIMA_LIVE_PROCESS_START_DECISION_CANDIDATE_SCHEMA,
        completionAuthority:
          "durable-inert-process-start-decision-not-effect-authority",
        completionSchema: COLIMA_LIVE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
        contractSchema:
          COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SCHEMA,
        evidenceClass: "production-pinned",
        intentCompletionSchema: COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA,
        intentContractSha256:
          COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
        intentOperationKind: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
        intentPublicationPlanSchema:
          COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
        operationKind: COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
        projectionSchema: COLIMA_LIVE_COMPLETED_PROVIDER_INTENT_PROJECTION_SCHEMA,
        publicationPlanSchema:
          COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
        rootObservationSchema: COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
      });
}

function operationContract(fixtureOnly) {
  const selected = variant(fixtureOnly);
  return deepFreeze({
    action: COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
    cleanup_authorized: false,
    completed_intent_projection_schema: selected.projectionSchema,
    completion_schema: selected.completionSchema,
    cross_namespace_atomic_reservation: false,
    decision: "requested-not-executed-not-authorized",
    decision_candidate_schema: selected.candidateSchema,
    effect_execution_authorized: false,
    environment_publication_authorized: false,
    evidence_class: selected.evidenceClass,
    evidence_publication_authorized: false,
    finalization_authorized: false,
    fresh_admission_schema: selected.admissionSchema,
    future_effect_fresh_admission_required: true,
    lifecycle_exposed: false,
    operation_kind: selected.operationKind,
    process_signal_authorized: false,
    process_spawn_authorized: false,
    process_start_authorized: false,
    process_state: "not-reached",
    provider_adapter_execution_authorized: false,
    provider_artifact_publication_authorized: false,
    provider_class: COLIMA_LIVE_PROVIDER_CLASS,
    provider_recovery_authorized: false,
    provider_root_mutation_authorized: false,
    publication_plan_schema: selected.publicationPlanSchema,
    receipt_publication_authorized: false,
    recovery_disposition: "aborted-before-effect-only",
    runtime_publication_authorized: false,
    schema: selected.contractSchema,
    state_integration: COLIMA_LIVE_PROVIDER_START_DECISION_STATE_INTEGRATION,
    state_process_start_decision_publication_authorized: true,
    target_provider_intent_completion_schema: selected.intentCompletionSchema,
    target_provider_intent_operation_contract_sha256:
      selected.intentContractSha256,
    target_provider_intent_operation_kind: selected.intentOperationKind,
    target_provider_intent_publication_plan_schema:
      selected.intentPublicationPlanSchema,
    target_provider_intent_state_integration:
      COLIMA_LIVE_PROVIDER_INTENT_STATE_INTEGRATION,
  });
}

export const COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT =
  operationContract(false);
export const COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT);
export const COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT =
  operationContract(true);
export const COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT);

function sourceContext(value) {
  exactKeys(
    value,
    ["closeAuthority", "fixtureOnly", "intentCompletion", "intentPublicationPlan"],
    "live provider start decision source",
  );
  const selected = variant(value.fixtureOnly);
  if (value.closeAuthority !== "owner") {
    fail("live provider start decision source close authority was refused", 69);
  }
  try {
    validateColimaLiveProviderIntentPublicationPlan(value.intentPublicationPlan, {
      fixtureOnly: value.fixtureOnly,
    });
    validateColimaLiveProviderIntentCompletion(value.intentCompletion, {
      closeSha256: value.intentCompletion?.intent_close_sha256,
      fixtureOnly: value.fixtureOnly,
      publicationPlan: value.intentPublicationPlan,
      slotSha256: value.intentCompletion?.intent_slot_sha256,
    });
  } catch (error) {
    if (error instanceof LiveProviderIntentFailure) {
      fail("live provider start decision source intent was refused", error.exitStatus);
    }
    throw error;
  }
  return Object.freeze({
    operationPlan: value.intentPublicationPlan.provider_operation_plan,
    selected,
  });
}

export function validateColimaLiveCompletedProviderIntentProjectionStructure(
  value,
  source,
) {
  exactKeys(
    value,
    PROJECTION_FIELDS,
    "live provider completed intent projection",
  );
  const { selected } = sourceContext(source);
  const completion = source.intentCompletion;
  if (
    value.schema !== selected.projectionSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.provider_intent_operation_kind !== selected.intentOperationKind ||
    value.provider_intent_operation_contract_sha256 !==
      selected.intentContractSha256 ||
    value.provider_intent_slot_sha256 !== completion.intent_slot_sha256 ||
    value.provider_intent_close_sha256 !== completion.intent_close_sha256 ||
    value.provider_intent_slot_sha256 === value.provider_intent_close_sha256 ||
    value.provider_intent_publication_plan_sha256 !==
      completion.intent_publication_plan_sha256 ||
    value.completed_plan_projection_sha256 !==
      completion.completed_plan_projection_sha256 ||
    value.close_authority !== "owner" ||
    !nonzeroSha256(value.provider_intent_slot_sha256) ||
    !nonzeroSha256(value.provider_intent_close_sha256) ||
    !nonzeroSha256(value.provider_intent_publication_plan_sha256) ||
    !nonzeroSha256(value.completed_plan_projection_sha256)
  ) {
    fail("live provider completed intent projection was refused", 69);
  }
  return value;
}

export function buildColimaLiveCompletedProviderIntentProjectionStructure(source) {
  const { selected } = sourceContext(source);
  const completion = source.intentCompletion;
  const projection = {
    close_authority: "owner",
    completed_plan_projection_sha256:
      completion.completed_plan_projection_sha256,
    evidence_class: selected.evidenceClass,
    provider_intent_close_sha256: completion.intent_close_sha256,
    provider_intent_operation_contract_sha256: selected.intentContractSha256,
    provider_intent_operation_kind: selected.intentOperationKind,
    provider_intent_publication_plan_sha256:
      completion.intent_publication_plan_sha256,
    provider_intent_slot_sha256: completion.intent_slot_sha256,
    schema: selected.projectionSchema,
  };
  validateColimaLiveCompletedProviderIntentProjectionStructure(projection, source);
  return deepFreeze(projection);
}

function validateRootObservation(value, source) {
  const { operationPlan, selected } = sourceContext(source);
  exactKeys(value, ROOT_OBSERVATION_FIELDS, "live provider fresh root observation");
  exactKeys(
    value.planned_names,
    ["lima_instance", "provider_profile"],
    "live provider fresh planned names",
  );
  const roles = COLIMA_LIVE_MUTATION_SURFACE_ROLES;
  if (
    !Array.isArray(value.root_observations) ||
    value.root_observations.length !== roles.length
  ) {
    fail("live provider fresh root observation was refused", 69);
  }
  for (const [index, root] of value.root_observations.entries()) {
    exactKeys(root, ROOT_FIELDS, "live provider fresh root");
    if (
      root.role !== roles[index] ||
      !new Set(["foreign-collision", "observed-pristine"]).has(
        root.disposition,
      ) ||
      !nonzeroSha256(root.namespace_identity_hmac_sha256) ||
      !nonzeroSha256(root.observed_entry_set_hmac_sha256)
    ) {
      fail("live provider fresh root was refused", 69);
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
      source.intentPublicationPlan.admission.root_observation
        .requirements_sha256 ||
    value.preparation_observation_sha256 !==
      operationPlan.preparation_observation_sha256 ||
    value.planned_names.provider_profile !== operationPlan.provider_profile ||
    value.planned_names.lima_instance !== `colima-${operationPlan.provider_profile}` ||
    operationPlan.provider_resource !== operationPlan.provider_profile
  ) {
    fail("live provider fresh root observation binding was refused", 69);
  }
  return value;
}

function supervisorLabel(source) {
  const label = source.intentPublicationPlan.admission.supervisor_process_group_label;
  if (
    !/^sv-c45-colima-pg-[0-9a-f]{32}-[0-9a-f]{12}$/u.test(label) ||
    Buffer.byteLength(label, "ascii") > 63
  ) {
    fail("live provider start decision supervisor label was refused", 69);
  }
  return label;
}

export function validateColimaLiveProviderStartDecisionCandidateStructure(
  value,
  expected,
) {
  exactKeys(
    expected,
    ["completedIntentProjection", "rootObservation", "source"],
    "live provider start decision candidate expectation",
  );
  const { selected } = sourceContext(expected.source);
  validateColimaLiveCompletedProviderIntentProjectionStructure(
    expected.completedIntentProjection,
    expected.source,
  );
  validateRootObservation(expected.rootObservation, expected.source);
  exactKeys(value, CANDIDATE_FIELDS, "live provider start decision candidate");
  if (
    expected.rootObservation.root_set_disposition !== "observed-pristine" ||
    value.schema !== selected.candidateSchema ||
    value.effect_name !== "provider-process-start" ||
    value.decision !== "requested-not-executed-not-authorized" ||
    value.process_state !== "not-reached" ||
    value.process_start_authorized !== false ||
    value.future_effect_fresh_admission_required !== true ||
    value.cross_namespace_atomic_reservation !== false ||
    value.completed_intent_projection_sha256 !==
      valueDigest(expected.completedIntentProjection) ||
    value.root_observation_sha256 !== valueDigest(expected.rootObservation) ||
    value.supervisor_process_group_label !== supervisorLabel(expected.source)
  ) {
    fail("live provider start decision candidate was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderStartDecisionCandidateStructure(value) {
  exactKeys(
    value,
    ["completedIntentProjection", "rootObservation", "source"],
    "live provider start decision candidate input",
  );
  const { selected } = sourceContext(value.source);
  validateColimaLiveCompletedProviderIntentProjectionStructure(
    value.completedIntentProjection,
    value.source,
  );
  validateRootObservation(value.rootObservation, value.source);
  const candidate = {
    completed_intent_projection_sha256: valueDigest(
      value.completedIntentProjection,
    ),
    cross_namespace_atomic_reservation: false,
    decision: "requested-not-executed-not-authorized",
    effect_name: "provider-process-start",
    future_effect_fresh_admission_required: true,
    process_start_authorized: false,
    process_state: "not-reached",
    root_observation_sha256: valueDigest(value.rootObservation),
    schema: selected.candidateSchema,
    supervisor_process_group_label: supervisorLabel(value.source),
  };
  validateColimaLiveProviderStartDecisionCandidateStructure(candidate, value);
  return deepFreeze(candidate);
}

export function validateColimaLiveProviderStartFreshAdmissionStructure(
  value,
  source,
) {
  const { selected } = sourceContext(source);
  exactKeys(value, ADMISSION_FIELDS, "live provider start fresh admission");
  validateColimaLiveCompletedProviderIntentProjectionStructure(
    value.completed_intent_projection,
    source,
  );
  validateRootObservation(value.root_observation, source);
  const expectedLabel = supervisorLabel(source);
  if (
    value.schema !== selected.admissionSchema ||
    value.authority !== selected.admissionAuthority ||
    value.supervisor_process_group_label !== expectedLabel
  ) {
    fail("live provider start fresh admission was refused", 69);
  }
  if (value.root_observation.root_set_disposition === "observed-pristine") {
    validateColimaLiveProviderStartDecisionCandidateStructure(
      value.process_start_decision_candidate,
      {
        completedIntentProjection: value.completed_intent_projection,
        rootObservation: value.root_observation,
        source,
      },
    );
  } else if (value.process_start_decision_candidate !== null) {
    fail("live provider collision admitted a start decision", 69);
  }
  return value;
}

export function buildColimaLiveProviderStartFreshAdmissionStructure(value) {
  exactKeys(
    value,
    ["completedIntentProjection", "rootObservation", "source"],
    "live provider start fresh admission input",
  );
  const { selected } = sourceContext(value.source);
  validateColimaLiveCompletedProviderIntentProjectionStructure(
    value.completedIntentProjection,
    value.source,
  );
  validateRootObservation(value.rootObservation, value.source);
  const candidate =
    value.rootObservation.root_set_disposition === "observed-pristine"
      ? buildColimaLiveProviderStartDecisionCandidateStructure(value)
      : null;
  const admission = {
    authority: selected.admissionAuthority,
    completed_intent_projection: value.completedIntentProjection,
    process_start_decision_candidate: candidate,
    root_observation: value.rootObservation,
    schema: selected.admissionSchema,
    supervisor_process_group_label: supervisorLabel(value.source),
  };
  validateColimaLiveProviderStartFreshAdmissionStructure(admission, value.source);
  return deepFreeze(admission);
}

export function validateColimaLiveProviderStartDecisionPublicationPlan(
  value,
  source,
) {
  const { operationPlan, selected } = sourceContext(source);
  exactKeys(
    value,
    PUBLICATION_PLAN_FIELDS,
    "live provider start decision publication plan",
  );
  validateColimaLiveProviderStartFreshAdmissionStructure(value.admission, source);
  const candidate = value.admission.process_start_decision_candidate;
  if (
    candidate === null ||
    value.admission.root_observation.root_set_disposition !== "observed-pristine" ||
    value.schema !== selected.publicationPlanSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.fixture_id !== operationPlan.fixture_id ||
    value.operation_kind !== selected.operationKind ||
    value.operation_contract_sha256 !==
      (source.fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256
        : COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256) ||
    value.admission_sha256 !== valueDigest(value.admission) ||
    value.completed_intent_projection_sha256 !==
      valueDigest(value.admission.completed_intent_projection) ||
    value.root_observation_sha256 !==
      valueDigest(value.admission.root_observation) ||
    value.decision_candidate_sha256 !== valueDigest(candidate) ||
    value.publication_claim !==
      "state-owner-reconstructs-equal-fresh-admission-at-each-publication-boundary" ||
    value.state_integration !==
      COLIMA_LIVE_PROVIDER_START_DECISION_STATE_INTEGRATION
  ) {
    fail("live provider start decision publication plan was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderStartDecisionPublicationPlan(value) {
  exactKeys(
    value,
    ["admission", "source"],
    "live provider start decision publication plan input",
  );
  const { operationPlan, selected } = sourceContext(value.source);
  validateColimaLiveProviderStartFreshAdmissionStructure(
    value.admission,
    value.source,
  );
  const candidate = value.admission.process_start_decision_candidate;
  if (candidate === null) {
    fail("live provider collision cannot become a start decision plan", 73);
  }
  const publicationPlan = {
    admission: value.admission,
    admission_sha256: valueDigest(value.admission),
    completed_intent_projection_sha256: valueDigest(
      value.admission.completed_intent_projection,
    ),
    decision_candidate_sha256: valueDigest(candidate),
    evidence_class: selected.evidenceClass,
    fixture_id: operationPlan.fixture_id,
    operation_contract_sha256: value.source.fixtureOnly
      ? COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256
      : COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
    operation_kind: selected.operationKind,
    publication_claim:
      "state-owner-reconstructs-equal-fresh-admission-at-each-publication-boundary",
    root_observation_sha256: valueDigest(value.admission.root_observation),
    schema: selected.publicationPlanSchema,
    state_integration:
      COLIMA_LIVE_PROVIDER_START_DECISION_STATE_INTEGRATION,
  };
  validateColimaLiveProviderStartDecisionPublicationPlan(
    publicationPlan,
    value.source,
  );
  return deepFreeze(publicationPlan);
}

export function validateColimaLiveProviderStartDecisionCompletion(
  value,
  expected,
) {
  exactKeys(
    expected,
    ["closeSha256", "publicationPlan", "slotSha256", "source"],
    "live provider start decision completion expectation",
  );
  const { selected } = sourceContext(expected.source);
  validateColimaLiveProviderStartDecisionPublicationPlan(
    expected.publicationPlan,
    expected.source,
  );
  exactKeys(value, COMPLETION_FIELDS, "live provider start decision completion");
  const candidate = expected.publicationPlan.admission.process_start_decision_candidate;
  if (
    !nonzeroSha256(expected.closeSha256) ||
    !nonzeroSha256(expected.slotSha256) ||
    expected.closeSha256 === expected.slotSha256 ||
    value.schema !== selected.completionSchema ||
    value.authority !== selected.completionAuthority ||
    value.evidence_class !== selected.evidenceClass ||
    value.completed_intent_projection_sha256 !==
      expected.publicationPlan.completed_intent_projection_sha256 ||
    value.decision_candidate_sha256 !== valueDigest(candidate) ||
    value.decision_publication_plan_sha256 !==
      valueDigest(expected.publicationPlan) ||
    value.decision_close_sha256 !== expected.closeSha256 ||
    value.decision_slot_sha256 !== expected.slotSha256
  ) {
    fail("live provider start decision completion was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderStartDecisionCompletion(value) {
  exactKeys(
    value,
    ["closeSha256", "publicationPlan", "slotSha256", "source"],
    "live provider start decision completion input",
  );
  const { selected } = sourceContext(value.source);
  validateColimaLiveProviderStartDecisionPublicationPlan(
    value.publicationPlan,
    value.source,
  );
  const candidate = value.publicationPlan.admission.process_start_decision_candidate;
  const completion = {
    authority: selected.completionAuthority,
    completed_intent_projection_sha256:
      value.publicationPlan.completed_intent_projection_sha256,
    decision_candidate_sha256: valueDigest(candidate),
    decision_close_sha256: value.closeSha256,
    decision_publication_plan_sha256: valueDigest(value.publicationPlan),
    decision_slot_sha256: value.slotSha256,
    evidence_class: selected.evidenceClass,
    schema: selected.completionSchema,
  };
  validateColimaLiveProviderStartDecisionCompletion(completion, value);
  return deepFreeze(completion);
}
