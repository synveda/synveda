#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
} from "./clean-engine-colima-live-schemas.mjs";
import {
  COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA,
  COLIMA_LIVE_PROVIDER_PLAN_STATE_INTEGRATION,
  LiveProviderPlanFailure,
  liveProviderPlanBytes,
  liveProviderPlanDigest,
  validateColimaLiveProviderOperationPlan,
} from "./clean-engine-live-provider-plan.mjs";
import {
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_CREATE_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_CLASS,
} from "./clean-engine-provider-adapter-registry.mjs";

export const COLIMA_LIVE_PLAN_COMPLETION_PROJECTION_SCHEMA =
  "synveda.clean-engine.colima-live-plan-completion-projection.v1";
export const COLIMA_LIVE_EFFECT_INTENT_CANDIDATE_SCHEMA =
  "synveda.clean-engine.colima-live-effect-intent-candidate.v1";
export const COLIMA_LIVE_EMPTY_PRE_EFFECT_PREFIX_SCHEMA =
  "synveda.clean-engine.colima-live-empty-pre-effect-prefix.v1";
export const COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA =
  "synveda.clean-engine.colima-live-pre-effect-admission.v1";
export const COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-pre-effect-admission.v1";
export const COLIMA_LIVE_PROVIDER_INTENT_ACTION = "provider-intent";
export const COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND =
  "colima-live-provider-intent-publication-v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND =
  "colima-live-fixture-provider-intent-publication-v1";
export const COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-provider-intent-operation-contract.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-intent-operation-contract.v1";
export const COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-provider-intent-publication-plan.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-intent-publication-plan.v1";
export const COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA =
  "synveda.clean-engine.colima-live-provider-intent-completion.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-intent-completion.v1";
export const COLIMA_LIVE_PROVIDER_INTENT_STATE_INTEGRATION =
  "mutation-journal-v6-inert-intent-only";

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
const ADMISSION_FIELDS = Object.freeze([
  "authority",
  "completion_projection",
  "intent_candidate",
  "pre_effect_prefix",
  "root_observation",
  "schema",
  "supervisor_process_group_label",
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
const PUBLICATION_PLAN_FIELDS = Object.freeze([
  "admission",
  "admission_sha256",
  "evidence_class",
  "fixture_id",
  "operation_contract_sha256",
  "operation_kind",
  "provider_operation_plan",
  "provider_operation_plan_sha256",
  "publication_claim",
  "root_observation_sha256",
  "schema",
  "state_integration",
]);
const COMPLETION_FIELDS = Object.freeze([
  "authority",
  "completed_plan_projection_sha256",
  "evidence_class",
  "intent_close_sha256",
  "intent_publication_plan_sha256",
  "intent_slot_sha256",
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

function intentOperationContract({
  evidenceClass,
  operationKind,
  publicationPlanSchema,
  schema,
}) {
  return deepFreeze({
    action: COLIMA_LIVE_PROVIDER_INTENT_ACTION,
    cleanup_authorized: false,
    effect_execution_authorized: false,
    evidence_class: evidenceClass,
    finalization_authorized: false,
    lifecycle_exposed: false,
    operation_kind: operationKind,
    provider_class: COLIMA_LIVE_PROVIDER_CLASS,
    provider_recovery_authorized: false,
    publication_plan_schema: publicationPlanSchema,
    receipt_publication_authorized: false,
    recovery_disposition: "aborted-before-effect-only",
    schema,
    state_integration: COLIMA_LIVE_PROVIDER_INTENT_STATE_INTEGRATION,
    state_intent_publication_authorized: true,
    target_create_operation_contract_sha256:
      COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
    target_create_operation_kind: COLIMA_LIVE_CREATE_OPERATION_KIND,
    target_provider_plan_schema: COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA,
    target_provider_plan_state_integration:
      COLIMA_LIVE_PROVIDER_PLAN_STATE_INTEGRATION,
  });
}

export const COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT =
  intentOperationContract({
    evidenceClass: "production-pinned",
    operationKind: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
    publicationPlanSchema:
      COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
    schema: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SCHEMA,
  });
export const COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT);
export const COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT =
  intentOperationContract({
    evidenceClass: "fixture-only",
    operationKind: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
    publicationPlanSchema:
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
    schema: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SCHEMA,
  });
export const COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT);

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

function intentPublicationVariant(fixtureOnly) {
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider intent publication variant was refused", 70);
  }
  return fixtureOnly
    ? Object.freeze({
        admissionAuthority:
          "fixture-only-point-in-time-not-effect-authority",
        admissionSchema: COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
        contractSha256:
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
        completionSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
        evidenceClass: "fixture-only",
        operationKind: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
        publicationPlanSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
        rootObservationSchema:
          COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
      })
    : Object.freeze({
        admissionAuthority: "point-in-time-not-effect-authority",
        admissionSchema: COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
        contractSha256:
          COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
        completionSchema: COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA,
        evidenceClass: "production-pinned",
        operationKind: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
        publicationPlanSchema:
          COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
        rootObservationSchema:
          COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
      });
}

function validateAdmissionRootObservation(value, operationPlan, fixtureOnly) {
  const variant = intentPublicationVariant(fixtureOnly);
  exactKeys(
    value,
    ROOT_OBSERVATION_FIELDS,
    "live provider intent root observation",
  );
  exactKeys(
    value.planned_names,
    ["lima_instance", "provider_profile"],
    "live provider intent planned names",
  );
  const roles = COLIMA_LIVE_MUTATION_SURFACE_ROLES;
  if (
    !Array.isArray(value.root_observations) ||
    value.root_observations.length !== roles.length
  ) {
    fail("live provider intent root observation was refused", 69);
  }
  for (const [index, root] of value.root_observations.entries()) {
    exactKeys(root, ROOT_FIELDS, "live provider intent root");
    if (
      root.role !== roles[index] ||
      root.disposition !== "observed-pristine" ||
      !nonzeroSha256(root.namespace_identity_hmac_sha256) ||
      !nonzeroSha256(root.observed_entry_set_hmac_sha256)
    ) {
      fail("live provider intent root was refused", 69);
    }
  }
  if (
    value.schema !== variant.rootObservationSchema ||
    value.evidence_class !== variant.evidenceClass ||
    value.root_set_disposition !== "observed-pristine" ||
    !nonzeroSha256(value.requirements_sha256) ||
    (!fixtureOnly &&
      value.requirements_sha256 !== operationPlan.requirements_sha256) ||
    value.preparation_observation_sha256 !==
      operationPlan.preparation_observation_sha256 ||
    value.planned_names.provider_profile !== operationPlan.provider_profile ||
    value.planned_names.lima_instance !==
      `colima-${operationPlan.provider_profile}`
  ) {
    fail("live provider intent root observation was refused", 69);
  }
  return value;
}

function validateAdmission(value, operationPlan, fixtureOnly) {
  const variant = intentPublicationVariant(fixtureOnly);
  exactKeys(value, ADMISSION_FIELDS, "live provider intent admission");
  validateColimaLivePlanCompletionProjectionStructure(
    value.completion_projection,
  );
  validateColimaLiveEffectIntentCandidateStructure(
    value.intent_candidate,
    value.completion_projection,
    operationPlan,
  );
  validateColimaLiveEmptyPreEffectPrefixStructure(
    value.pre_effect_prefix,
    value.intent_candidate,
    value.completion_projection,
    operationPlan,
  );
  validateAdmissionRootObservation(
    value.root_observation,
    operationPlan,
    fixtureOnly,
  );
  const expectedLabel =
    `sv-c45-colima-pg-${operationPlan.fixture_id}-` +
    value.completion_projection.plan_slot_sha256.slice(0, 12);
  if (
    value.schema !== variant.admissionSchema ||
    value.authority !== variant.admissionAuthority ||
    value.supervisor_process_group_label !== expectedLabel ||
    !/^sv-c45-colima-pg-[0-9a-f]{32}-[0-9a-f]{12}$/u.test(expectedLabel) ||
    Buffer.byteLength(expectedLabel, "ascii") > 63
  ) {
    fail("live provider intent admission was refused", 69);
  }
  return value;
}

export function validateColimaLiveProviderIntentPublicationPlan(
  value,
  options,
) {
  let fixtureOnly = false;
  if (options !== undefined) {
    exactKeys(
      options,
      ["fixtureOnly"],
      "live provider intent publication validation options",
    );
    if (typeof options.fixtureOnly !== "boolean") {
      fail("live provider intent publication validation options were refused", 70);
    }
    fixtureOnly = options.fixtureOnly;
  }
  const variant = intentPublicationVariant(fixtureOnly);
  const operationPlan = value?.provider_operation_plan;
  validateOperationPlan(operationPlan);
  exactKeys(
    value,
    PUBLICATION_PLAN_FIELDS,
    "live provider intent publication plan",
  );
  validateAdmission(value.admission, operationPlan, fixtureOnly);
  if (
    value.schema !== variant.publicationPlanSchema ||
    value.evidence_class !== variant.evidenceClass ||
    value.fixture_id !== operationPlan.fixture_id ||
    value.operation_kind !== variant.operationKind ||
    value.operation_contract_sha256 !== variant.contractSha256 ||
    value.provider_operation_plan_sha256 !==
      liveProviderPlanDigest(liveProviderPlanBytes(operationPlan)) ||
    value.admission_sha256 !== valueDigest(value.admission) ||
    value.root_observation_sha256 !==
      valueDigest(value.admission.root_observation) ||
    value.publication_claim !==
      "canonical-root-observation-equal-before-slot-link-after-slot-acquisition-before-close-link" ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_INTENT_STATE_INTEGRATION
  ) {
    fail("live provider intent publication plan was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderIntentPublicationPlan(value) {
  exactKeys(
    value,
    ["admission", "fixtureOnly", "operationPlan"],
    "live provider intent publication plan input",
  );
  const { admission, fixtureOnly, operationPlan } = value;
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider intent publication plan input was refused", 70);
  }
  const variant = intentPublicationVariant(fixtureOnly);
  validateOperationPlan(operationPlan);
  validateAdmission(admission, operationPlan, fixtureOnly);
  const publicationPlan = {
    admission,
    admission_sha256: valueDigest(admission),
    evidence_class: variant.evidenceClass,
    fixture_id: operationPlan.fixture_id,
    operation_contract_sha256: variant.contractSha256,
    operation_kind: variant.operationKind,
    provider_operation_plan: operationPlan,
    provider_operation_plan_sha256:
      liveProviderPlanDigest(liveProviderPlanBytes(operationPlan)),
    publication_claim:
      "canonical-root-observation-equal-before-slot-link-after-slot-acquisition-before-close-link",
    root_observation_sha256: valueDigest(admission.root_observation),
    schema: variant.publicationPlanSchema,
    state_integration: COLIMA_LIVE_PROVIDER_INTENT_STATE_INTEGRATION,
  };
  validateColimaLiveProviderIntentPublicationPlan(publicationPlan, {
    fixtureOnly,
  });
  return deepFreeze(publicationPlan);
}

export function buildColimaLiveProviderIntentCompletion(value) {
  exactKeys(
    value,
    ["closeSha256", "fixtureOnly", "publicationPlan", "slotSha256"],
    "live provider intent completion input",
  );
  const { closeSha256, fixtureOnly, publicationPlan, slotSha256 } = value;
  const completion = {
    authority:
      fixtureOnly === true
        ? "fixture-only-durable-inert-intent-not-effect-authority"
        : "durable-inert-intent-not-effect-authority",
    completed_plan_projection_sha256: valueDigest(
      publicationPlan?.admission?.completion_projection,
    ),
    evidence_class: fixtureOnly === true ? "fixture-only" : "production-pinned",
    intent_close_sha256: closeSha256,
    intent_publication_plan_sha256: valueDigest(publicationPlan),
    intent_slot_sha256: slotSha256,
    schema:
      fixtureOnly === true
        ? COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA
        : COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA,
  };
  validateColimaLiveProviderIntentCompletion(completion, value);
  return deepFreeze(completion);
}

export function validateColimaLiveProviderIntentCompletion(value, expected) {
  exactKeys(value, COMPLETION_FIELDS, "live provider intent completion");
  exactKeys(
    expected,
    ["closeSha256", "fixtureOnly", "publicationPlan", "slotSha256"],
    "live provider intent completion expectation",
  );
  const { closeSha256, fixtureOnly, publicationPlan, slotSha256 } = expected;
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider intent completion expectation was refused", 70);
  }
  const variant = intentPublicationVariant(fixtureOnly);
  validateColimaLiveProviderIntentPublicationPlan(publicationPlan, {
    fixtureOnly,
  });
  if (
    !nonzeroSha256(closeSha256) ||
    !nonzeroSha256(slotSha256) ||
    value.schema !== variant.completionSchema ||
    value.authority !==
      (fixtureOnly
        ? "fixture-only-durable-inert-intent-not-effect-authority"
        : "durable-inert-intent-not-effect-authority") ||
    value.evidence_class !== variant.evidenceClass ||
    value.completed_plan_projection_sha256 !==
      valueDigest(publicationPlan.admission.completion_projection) ||
    value.intent_publication_plan_sha256 !== valueDigest(publicationPlan) ||
    value.intent_close_sha256 !== closeSha256 ||
    value.intent_slot_sha256 !== slotSha256
  ) {
    fail("live provider intent completion was refused", 69);
  }
  return value;
}
