#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
} from "./clean-engine-colima-live-schemas.mjs";
import {
  LiveProviderProcessStartFailure,
  liveProviderProcessStartBytes,
  liveProviderProcessStartDigest,
  validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
} from "./clean-engine-live-provider-process-start.mjs";

export const COLIMA_LIVE_PROVIDER_RESERVATION_ACTION =
  "provider-reservation";
export { COLIMA_LIVE_PROVIDER_RESERVATION_NAME };
export const COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_KIND =
  "colima-live-provider-reservation-publication-v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_KIND =
  "colima-live-fixture-provider-reservation-publication-v1";
export const COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-provider-reservation-operation-contract.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-reservation-operation-contract.v1";
export const COLIMA_LIVE_PROVIDER_RESERVATION_PUBLICATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-provider-reservation-publication-plan.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_PUBLICATION_PLAN_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-reservation-publication-plan.v1";
export const COLIMA_LIVE_PROVIDER_RESERVATION_WITNESS_SCHEMA =
  "synveda.clean-engine.colima-live-provider-reservation-witness.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_WITNESS_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-reservation-witness.v1";
export const COLIMA_LIVE_PROVIDER_RESERVATION_SETTLEMENT_SCHEMA =
  "synveda.clean-engine.colima-live-provider-reservation-settlement.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_SETTLEMENT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-reservation-settlement.v1";
export const COLIMA_LIVE_PROVIDER_RESERVATION_COMPLETION_SCHEMA =
  "synveda.clean-engine.colima-live-provider-reservation-completion.v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-reservation-completion.v1";
export const COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION =
  "mutation-journal-v6-no-spawn-reservation-only";

const ZERO_SHA256 = "0".repeat(64);
const IDENTITY_FIELDS = Object.freeze([
  "device",
  "inode",
  "mode",
  "uid",
]);
const NAMESPACE_BINDING_FIELDS = Object.freeze([
  ...IDENTITY_FIELDS,
  "namespace_identity_hmac_sha256",
  "observed_entry_set_hmac_sha256",
  "role",
]);
const CONTRACT_CAPABILITY_FIELDS = Object.freeze([
  "cleanup_authorized",
  "environment_publication_authorized",
  "evidence_publication_authorized",
  "finalization_authorized",
  "general_provider_root_mutation_authorized",
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
  "receipt_publication_authorized",
  "reservation_recovery_authorized",
  "runtime_publication_authorized",
  "state_evidence_publication_authorized",
]);
const CONTRACT_FIELDS = Object.freeze([
  "action",
  "authority",
  "capabilities",
  "evidence_class",
  "fixed_marker_name",
  "operation_kind",
  "reservation_scope",
  "schema",
  "state_integration",
  "threat_boundary",
]);
const PLAN_FIELDS = Object.freeze([
  "admission",
  "admission_sha256",
  "completed_start_decision_projection_sha256",
  "evidence_class",
  "fixture_id",
  "fixed_marker_name",
  "namespace_bindings",
  "operation_contract_sha256",
  "operation_kind",
  "provider_root_identity",
  "reservation_authorization",
  "schema",
  "state_integration",
  "state_run_identity",
]);
const WITNESS_FIELDS = Object.freeze([
  "evidence_class",
  "fixture_id",
  "fixed_marker_name",
  "marker_link_count",
  "operation_contract_sha256",
  "operation_kind",
  "operation_plan_sha256",
  "process_attempt",
  "reservation_disposition",
  "schema",
  "slot_sequence",
  "slot_sha256",
  "state_integration",
]);
const SETTLEMENT_FIELDS = Object.freeze([
  "authority",
  "authority_sha256",
  "environment_publication_authorized",
  "evidence_class",
  "fixture_id",
  "fixed_marker_name",
  "marker_retirement",
  "operation_contract_sha256",
  "operation_kind",
  "operation_plan_sha256",
  "pre_retirement_root_observation_sha256",
  "process_attempt",
  "process_start_authorized",
  "receipt_publication_authorized",
  "reservation_disposition",
  "reservation_witness_sha256",
  "schema",
  "slot_sequence",
  "slot_sha256",
  "state_integration",
]);
const COMPLETION_FIELDS = Object.freeze([
  "close_sha256",
  "evidence_class",
  "fixture_id",
  "operation_contract_sha256",
  "operation_kind",
  "operation_plan_sha256",
  "reservation_settlement_sha256",
  "reservation_witness_sha256",
  "schema",
  "slot_sequence",
  "slot_sha256",
  "state_integration",
]);

export class LiveProviderReservationFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderReservationFailure(message, exitStatus);
}

function closedDataEntries(value, label) {
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key !== "string")) {
    fail(`${label} contained a symbol property`, 70);
  }
  if (Array.isArray(value)) {
    const descriptor = Object.getOwnPropertyDescriptor(value, "length");
    if (
      Object.getPrototypeOf(value) !== Array.prototype ||
      descriptor === undefined ||
      !("value" in descriptor) ||
      !Number.isSafeInteger(descriptor.value) ||
      descriptor.value < 0 ||
      keys.length !== descriptor.value + 1
    ) {
      fail(`${label} was not a dense data array`, 70);
    }
    const entries = [];
    for (const key of keys) {
      if (key === "length") continue;
      const child = Object.getOwnPropertyDescriptor(value, key);
      const index = Number(key);
      if (
        !/^(?:0|[1-9][0-9]*)$/u.test(key) ||
        !Number.isSafeInteger(index) ||
        index >= descriptor.value ||
        child === undefined ||
        !("value" in child) ||
        child.enumerable !== true
      ) {
        fail(`${label} was not a dense data array`, 70);
      }
      entries.push([index, child.value]);
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
    const inspected = closedDataEntries(value, "live provider reservation canonical value");
    if (inspected.kind === "array") {
      return `[${inspected.entries.map(([, child]) => canonical(child)).join(",")}]`;
    }
    return `{${inspected.entries
      .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
      .map(([key, child]) => `${JSON.stringify(key)}:${canonical(child)}`)
      .join(",")}}`;
  }
  fail("live provider reservation canonical value was refused", 70);
}

export function liveProviderReservationBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderReservationDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function valueDigest(value) {
  return liveProviderReservationDigest(liveProviderReservationBytes(value));
}

function processValueDigest(value) {
  return liveProviderProcessStartDigest(liveProviderProcessStartBytes(value));
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object") {
    for (const [, child] of closedDataEntries(
      value,
      "live provider reservation freeze value",
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

function lowerHex(value, length) {
  return (
    typeof value === "string" &&
    value.length === length &&
    /^[0-9a-f]+$/u.test(value)
  );
}

function nonzeroSha256(value) {
  return lowerHex(value, 64) && value !== ZERO_SHA256;
}

function decimal(value, { positive = false } = {}) {
  if (typeof value !== "string" || !/^(?:0|[1-9][0-9]*)$/u.test(value)) {
    return false;
  }
  const parsed = BigInt(value);
  return !positive || parsed > 0n;
}

function variant(fixtureOnly) {
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider reservation variant was refused", 70);
  }
  return fixtureOnly
    ? Object.freeze({
        completionSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
        contractSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SCHEMA,
        evidenceClass: "fixture-only",
        operationKind:
          COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_KIND,
        planSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_PUBLICATION_PLAN_SCHEMA,
        settlementSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_SETTLEMENT_SCHEMA,
        witnessSchema:
          COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_WITNESS_SCHEMA,
      })
    : Object.freeze({
        completionSchema: COLIMA_LIVE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
        contractSchema:
          COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SCHEMA,
        evidenceClass: "production-pinned",
        operationKind: COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_KIND,
        planSchema: COLIMA_LIVE_PROVIDER_RESERVATION_PUBLICATION_PLAN_SCHEMA,
        settlementSchema: COLIMA_LIVE_PROVIDER_RESERVATION_SETTLEMENT_SCHEMA,
        witnessSchema: COLIMA_LIVE_PROVIDER_RESERVATION_WITNESS_SCHEMA,
      });
}

function processSourceContext(source) {
  exactKeys(
    source,
    [
      "closeAuthority",
      "fixtureOnly",
      "startDecisionCompletion",
      "startDecisionPublicationPlan",
      "startDecisionSource",
    ],
    "live provider reservation source",
  );
  const selected = variant(source.fixtureOnly);
  let admissionSourceValid = false;
  try {
    const operationPlan =
      source.startDecisionSource?.intentPublicationPlan?.provider_operation_plan;
    admissionSourceValid =
      source.closeAuthority === "owner" &&
      source.startDecisionSource?.fixtureOnly === source.fixtureOnly &&
      operationPlan?.fixture_id !== undefined;
  } catch {
    admissionSourceValid = false;
  }
  if (!admissionSourceValid) {
    fail("live provider reservation source was refused", 69);
  }
  return Object.freeze({
    fixtureId:
      source.startDecisionSource.intentPublicationPlan.provider_operation_plan
        .fixture_id,
    selected,
  });
}

function validateAdmission(admission, source) {
  try {
    validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
      admission,
      source,
    );
  } catch (error) {
    if (error instanceof LiveProviderProcessStartFailure) {
      fail("live provider reservation admission was refused", error.exitStatus);
    }
    throw error;
  }
  if (
    admission.root_observation.root_set_disposition !== "observed-pristine" ||
    admission.process_start_effect_candidate === null ||
    admission.process_start_effect_candidate.process_start_authorized !== false ||
    admission.process_start_effect_candidate.process_spawn_authorized !== false ||
    admission.process_start_effect_candidate.process_signal_authorized !== false ||
    admission.process_start_effect_candidate.provider_adapter_execution_authorized !==
      false
  ) {
    fail("live provider reservation admission was not pristine and inert", 73);
  }
  return admission;
}

function validateIdentity(value, label) {
  exactKeys(value, IDENTITY_FIELDS, label);
  if (
    !decimal(value.device) ||
    !decimal(value.inode, { positive: true }) ||
    value.mode !== "0700" ||
    !decimal(value.uid)
  ) {
    fail(`${label} was refused`, 69);
  }
  return value;
}

function validateNamespaceBindings(value, admission, commonDevice, commonUid) {
  if (
    !Array.isArray(value) ||
    value.length !== COLIMA_LIVE_MUTATION_SURFACE_ROLES.length
  ) {
    fail("live provider reservation namespace bindings were refused", 69);
  }
  for (const [index, binding] of value.entries()) {
    exactKeys(
      binding,
      NAMESPACE_BINDING_FIELDS,
      "live provider reservation namespace binding",
    );
    const observed = admission.root_observation.root_observations[index];
    if (
      binding.role !== COLIMA_LIVE_MUTATION_SURFACE_ROLES[index] ||
      binding.role !== observed.role ||
      binding.device !== commonDevice ||
      binding.uid !== commonUid ||
      binding.mode !== "0700" ||
      !decimal(binding.inode, { positive: true }) ||
      binding.namespace_identity_hmac_sha256 !==
        observed.namespace_identity_hmac_sha256 ||
      binding.observed_entry_set_hmac_sha256 !==
        observed.observed_entry_set_hmac_sha256 ||
      !nonzeroSha256(binding.namespace_identity_hmac_sha256) ||
      !nonzeroSha256(binding.observed_entry_set_hmac_sha256)
    ) {
      fail("live provider reservation namespace binding was refused", 69);
    }
  }
  return value;
}

function operationContract(fixtureOnly) {
  const selected = variant(fixtureOnly);
  return deepFreeze({
    action: COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
    authority: "state-owner-fresh-admission-exact-marker-only",
    capabilities: {
      cleanup_authorized: false,
      environment_publication_authorized: false,
      evidence_publication_authorized: false,
      finalization_authorized: false,
      general_provider_root_mutation_authorized: false,
      lifecycle_exposed: false,
      marker_link_authorized: true,
      marker_retirement_authorized: true,
      process_group_ownership_authorized: false,
      process_signal_authorized: false,
      process_spawn_authorized: false,
      process_start_authorized: false,
      provider_adapter_execution_authorized: false,
      provider_artifact_publication_authorized: false,
      provider_recovery_authorized: false,
      receipt_publication_authorized: false,
      reservation_recovery_authorized: true,
      runtime_publication_authorized: false,
      state_evidence_publication_authorized: true,
    },
    evidence_class: selected.evidenceClass,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    operation_kind: selected.operationKind,
    reservation_scope:
      "one-provider-root-six-fixed-child-namespaces-and-one-state-run",
    schema: selected.contractSchema,
    state_integration: COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION,
    threat_boundary: "cooperative-same-uid-single-host",
  });
}

function validateOperationContract(value, fixtureOnly) {
  const expected = operationContract(fixtureOnly);
  exactKeys(value, CONTRACT_FIELDS, "live provider reservation operation contract");
  exactKeys(
    value.capabilities,
    CONTRACT_CAPABILITY_FIELDS,
    "live provider reservation capabilities",
  );
  if (!liveProviderReservationBytes(value).equals(liveProviderReservationBytes(expected))) {
    fail("live provider reservation operation contract was refused", 69);
  }
  return value;
}

export const COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT =
  operationContract(false);
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT =
  operationContract(true);
export const COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT);
export const COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT);

function expectedContractSha256(fixtureOnly) {
  return fixtureOnly
    ? COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256
    : COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256;
}

export function validateColimaLiveProviderReservationPublicationPlan(
  value,
  source,
) {
  const { fixtureId, selected } = processSourceContext(source);
  exactKeys(value, PLAN_FIELDS, "live provider reservation publication plan");
  validateAdmission(value.admission, source);
  validateIdentity(
    value.provider_root_identity,
    "live provider reservation provider-root identity",
  );
  validateIdentity(
    value.state_run_identity,
    "live provider reservation state-run identity",
  );
  validateNamespaceBindings(
    value.namespace_bindings,
    value.admission,
    value.provider_root_identity.device,
    value.provider_root_identity.uid,
  );
  const directoryIdentities = [
    value.state_run_identity,
    value.provider_root_identity,
    ...value.namespace_bindings,
  ].map((identity) => `${identity.device}:${identity.inode}`);
  if (
    value.schema !== selected.planSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.fixture_id !== fixtureId ||
    value.fixed_marker_name !== COLIMA_LIVE_PROVIDER_RESERVATION_NAME ||
    value.operation_kind !== selected.operationKind ||
    value.operation_contract_sha256 !==
      expectedContractSha256(source.fixtureOnly) ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION ||
    value.reservation_authorization !==
      "state-owner-fresh-check-and-single-link-cas-required" ||
    value.admission_sha256 !== processValueDigest(value.admission) ||
    value.completed_start_decision_projection_sha256 !==
      processValueDigest(value.admission.completed_start_decision_projection) ||
    value.state_run_identity.device !== value.provider_root_identity.device ||
    value.state_run_identity.uid !== value.provider_root_identity.uid ||
    new Set(directoryIdentities).size !== directoryIdentities.length
  ) {
    fail("live provider reservation publication plan was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderReservationPublicationPlan({
  admission,
  namespaceBindings,
  providerRootIdentity,
  source,
  stateRunIdentity,
}) {
  exactKeys(
    {
      admission,
      namespaceBindings,
      providerRootIdentity,
      source,
      stateRunIdentity,
    },
    [
      "admission",
      "namespaceBindings",
      "providerRootIdentity",
      "source",
      "stateRunIdentity",
    ],
    "live provider reservation publication-plan input",
  );
  const { fixtureId, selected } = processSourceContext(source);
  validateAdmission(admission, source);
  const plan = {
    admission,
    admission_sha256: processValueDigest(admission),
    completed_start_decision_projection_sha256: processValueDigest(
      admission.completed_start_decision_projection,
    ),
    evidence_class: selected.evidenceClass,
    fixture_id: fixtureId,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    namespace_bindings: namespaceBindings,
    operation_contract_sha256: expectedContractSha256(source.fixtureOnly),
    operation_kind: selected.operationKind,
    provider_root_identity: providerRootIdentity,
    reservation_authorization:
      "state-owner-fresh-check-and-single-link-cas-required",
    schema: selected.planSchema,
    state_integration: COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION,
    state_run_identity: stateRunIdentity,
  };
  validateColimaLiveProviderReservationPublicationPlan(plan, source);
  return deepFreeze(plan);
}

function validateSlotBinding(value, publicationPlan) {
  if (
    !Number.isSafeInteger(value.slot_sequence) ||
    value.slot_sequence < 0 ||
    value.slot_sequence > 63 ||
    !nonzeroSha256(value.slot_sha256) ||
    value.operation_plan_sha256 !== valueDigest(publicationPlan)
  ) {
    fail("live provider reservation slot binding was refused", 69);
  }
}

export function validateColimaLiveProviderReservationWitness(
  value,
  { publicationPlan, source },
) {
  const { fixtureId, selected } = processSourceContext(source);
  validateColimaLiveProviderReservationPublicationPlan(publicationPlan, source);
  exactKeys(value, WITNESS_FIELDS, "live provider reservation witness");
  validateSlotBinding(value, publicationPlan);
  if (
    value.schema !== selected.witnessSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.fixture_id !== fixtureId ||
    value.fixed_marker_name !== COLIMA_LIVE_PROVIDER_RESERVATION_NAME ||
    value.marker_link_count !== 2 ||
    value.operation_kind !== selected.operationKind ||
    value.operation_contract_sha256 !==
      expectedContractSha256(source.fixtureOnly) ||
    value.process_attempt !== "not-started" ||
    value.reservation_disposition !== "held-no-process" ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION
  ) {
    fail("live provider reservation witness was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderReservationWitness({
  publicationPlan,
  slotSequence,
  slotSha256,
  source,
}) {
  const { fixtureId, selected } = processSourceContext(source);
  validateColimaLiveProviderReservationPublicationPlan(publicationPlan, source);
  const witness = {
    evidence_class: selected.evidenceClass,
    fixture_id: fixtureId,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    marker_link_count: 2,
    operation_contract_sha256: expectedContractSha256(source.fixtureOnly),
    operation_kind: selected.operationKind,
    operation_plan_sha256: valueDigest(publicationPlan),
    process_attempt: "not-started",
    reservation_disposition: "held-no-process",
    schema: selected.witnessSchema,
    slot_sequence: slotSequence,
    slot_sha256: slotSha256,
    state_integration: COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION,
  };
  validateColimaLiveProviderReservationWitness(witness, {
    publicationPlan,
    source,
  });
  return deepFreeze(witness);
}

export function validateColimaLiveProviderReservationSettlement(
  value,
  { publicationPlan, slotSequence, slotSha256, source, witness },
) {
  const { fixtureId, selected } = processSourceContext(source);
  validateColimaLiveProviderReservationPublicationPlan(publicationPlan, source);
  validateColimaLiveProviderReservationWitness(witness, {
    publicationPlan,
    source,
  });
  exactKeys(value, SETTLEMENT_FIELDS, "live provider reservation settlement");
  if (
    value.schema !== selected.settlementSchema ||
    !new Set(["owner", "recovery"]).has(value.authority) ||
    !nonzeroSha256(value.authority_sha256) ||
    value.evidence_class !== selected.evidenceClass ||
    value.fixture_id !== fixtureId ||
    value.fixed_marker_name !== COLIMA_LIVE_PROVIDER_RESERVATION_NAME ||
    value.operation_kind !== selected.operationKind ||
    value.operation_contract_sha256 !==
      expectedContractSha256(source.fixtureOnly) ||
    value.operation_plan_sha256 !== valueDigest(publicationPlan) ||
    value.slot_sequence !== slotSequence ||
    value.slot_sha256 !== slotSha256 ||
    witness.slot_sequence !== slotSequence ||
    witness.slot_sha256 !== slotSha256 ||
    value.reservation_witness_sha256 !== valueDigest(witness) ||
    value.pre_retirement_root_observation_sha256 !==
      processValueDigest(publicationPlan.admission.root_observation) ||
    value.process_attempt !== "not-started" ||
    value.process_start_authorized !== false ||
    value.receipt_publication_authorized !== false ||
    value.environment_publication_authorized !== false ||
    value.marker_retirement !== "authorized-exact-marker-link-only" ||
    value.reservation_disposition !== "retirement-authorized-without-process" ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION
  ) {
    fail("live provider reservation settlement was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderReservationSettlement({
  authority,
  authoritySha256,
  preRetirementRootObservation,
  publicationPlan,
  slotSequence,
  slotSha256,
  source,
  witness,
}) {
  const { fixtureId, selected } = processSourceContext(source);
  validateColimaLiveProviderReservationWitness(witness, {
    publicationPlan,
    source,
  });
  if (
    processValueDigest(preRetirementRootObservation) !==
    processValueDigest(publicationPlan.admission.root_observation)
  ) {
    fail("live provider reservation pre-retirement observation was refused", 73);
  }
  const settlement = {
    authority,
    authority_sha256: authoritySha256,
    environment_publication_authorized: false,
    evidence_class: selected.evidenceClass,
    fixture_id: fixtureId,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    marker_retirement: "authorized-exact-marker-link-only",
    operation_contract_sha256: expectedContractSha256(source.fixtureOnly),
    operation_kind: selected.operationKind,
    operation_plan_sha256: valueDigest(publicationPlan),
    pre_retirement_root_observation_sha256: processValueDigest(
      preRetirementRootObservation,
    ),
    process_attempt: "not-started",
    process_start_authorized: false,
    receipt_publication_authorized: false,
    reservation_disposition: "retirement-authorized-without-process",
    reservation_witness_sha256: valueDigest(witness),
    schema: selected.settlementSchema,
    slot_sequence: slotSequence,
    slot_sha256: slotSha256,
    state_integration: COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION,
  };
  validateColimaLiveProviderReservationSettlement(settlement, {
    publicationPlan,
    slotSequence,
    slotSha256,
    source,
    witness,
  });
  return deepFreeze(settlement);
}

export function validateColimaLiveProviderReservationCompletion(
  value,
  { closeSha256, publicationPlan, settlement, slotSequence, slotSha256, source, witness },
) {
  const { fixtureId, selected } = processSourceContext(source);
  validateColimaLiveProviderReservationSettlement(settlement, {
    publicationPlan,
    slotSequence,
    slotSha256,
    source,
    witness,
  });
  exactKeys(value, COMPLETION_FIELDS, "live provider reservation completion");
  if (
    value.schema !== selected.completionSchema ||
    value.evidence_class !== selected.evidenceClass ||
    value.fixture_id !== fixtureId ||
    value.operation_kind !== selected.operationKind ||
    value.operation_contract_sha256 !==
      expectedContractSha256(source.fixtureOnly) ||
    value.operation_plan_sha256 !== valueDigest(publicationPlan) ||
    value.reservation_witness_sha256 !== valueDigest(witness) ||
    value.reservation_settlement_sha256 !== valueDigest(settlement) ||
    value.slot_sequence !== slotSequence ||
    value.slot_sha256 !== slotSha256 ||
    value.close_sha256 !== closeSha256 ||
    !nonzeroSha256(value.close_sha256) ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION
  ) {
    fail("live provider reservation completion was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderReservationCompletion({
  closeSha256,
  publicationPlan,
  settlement,
  slotSequence,
  slotSha256,
  source,
  witness,
}) {
  const { fixtureId, selected } = processSourceContext(source);
  const completion = {
    close_sha256: closeSha256,
    evidence_class: selected.evidenceClass,
    fixture_id: fixtureId,
    operation_contract_sha256: expectedContractSha256(source.fixtureOnly),
    operation_kind: selected.operationKind,
    operation_plan_sha256: valueDigest(publicationPlan),
    reservation_settlement_sha256: valueDigest(settlement),
    reservation_witness_sha256: valueDigest(witness),
    schema: selected.completionSchema,
    slot_sequence: slotSequence,
    slot_sha256: slotSha256,
    state_integration: COLIMA_LIVE_PROVIDER_RESERVATION_STATE_INTEGRATION,
  };
  validateColimaLiveProviderReservationCompletion(completion, {
    closeSha256,
    publicationPlan,
    settlement,
    slotSequence,
    slotSha256,
    source,
    witness,
  });
  return deepFreeze(completion);
}

// Canonical values never carry authority when reconstructed by a caller. The
// state owner is the sole component permitted to compose the exact filesystem
// transition around a freshly reconstructed admission.
export function authorizeColimaLiveProviderReservationEffect(argumentsValue) {
  exactKeys(
    argumentsValue,
    ["publicationPlan", "source"],
    "live provider reservation authorization arguments",
  );
  validateColimaLiveProviderReservationPublicationPlan(
    argumentsValue.publicationPlan,
    argumentsValue.source,
  );
  fail("serialized live provider reservation data cannot authorize an effect", 69);
}

// Validate the constants at module load so a field edit cannot silently leave
// a stale exported digest.
validateOperationContract(
  COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT,
  false,
);
validateOperationContract(
  COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT,
  true,
);
