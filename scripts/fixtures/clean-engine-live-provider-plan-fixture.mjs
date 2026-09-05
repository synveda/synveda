import {
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_CREATE_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_CLASS,
  PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
  authorizeProviderAdapterPlanning,
} from "../../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs";
import {
  COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA,
  validateColimaLiveProviderOperationPlan,
} from "../../deploy/compose/scripts/clean-engine-live-provider-plan.mjs";
import { COLIMA_LIVE_OBSERVATION_SCHEMA } from "../../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";

export function cleanEngineLiveProviderOperationPlan({
  candidateSha256,
  fixtureId,
  observationSha256 = "e".repeat(64),
  sourceHeadSha256,
}) {
  const tuple = {
    action: "provider-create",
    operation_contract_sha256:
      COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
    operation_kind: COLIMA_LIVE_CREATE_OPERATION_KIND,
    provider_class: COLIMA_LIVE_PROVIDER_CLASS,
  };
  const resolution = authorizeProviderAdapterPlanning(tuple);
  const value = {
    action: resolution.action,
    adapter_key_sha256: resolution.key_sha256,
    capabilities: { ...PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES },
    evidence_schema: resolution.evidence_schema,
    fixture_id: fixtureId,
    operation_contract_sha256: resolution.operation_contract_sha256,
    operation_kind: resolution.operation_kind,
    preparation_observation_schema: COLIMA_LIVE_OBSERVATION_SCHEMA,
    preparation_observation_sha256: observationSha256,
    provider_class: resolution.provider_class,
    provider_profile: `synveda-cpr45-${fixtureId}`,
    provider_resource: `synveda-cpr45-${fixtureId}`,
    registry_sha256: resolution.registry_sha256,
    requirements_sha256: resolution.requirements_sha256,
    schema: COLIMA_LIVE_PROVIDER_OPERATION_PLAN_SCHEMA,
    source_candidate_sha256: candidateSha256,
    source_head_sha256: sourceHeadSha256,
    source_sequence: 0,
    state_integration: resolution.state_integration,
  };
  validateColimaLiveProviderOperationPlan(value);
  return structuredClone(value);
}
