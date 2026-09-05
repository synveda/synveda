import {
  COLIMA_LIVE_EMPTY_PRE_EFFECT_PREFIX_SCHEMA,
  buildColimaLiveEffectIntentCandidateStructure,
  buildColimaLivePlanCompletionProjectionStructure,
  liveProviderIntentBytes,
  liveProviderIntentDigest,
} from "../../deploy/compose/scripts/clean-engine-live-provider-intent.mjs";
import {
  liveProviderPlanBytes,
  liveProviderPlanDigest,
} from "../../deploy/compose/scripts/clean-engine-live-provider-plan.mjs";
import {
  cleanEngineLiveProviderOperationPlan,
} from "./clean-engine-live-provider-plan-fixture.mjs";

function digest(value) {
  return liveProviderIntentDigest(liveProviderIntentBytes(value));
}

export function cleanEngineLivePlanCompletionProjectionFixture() {
  const operationPlan = cleanEngineLiveProviderOperationPlan({
    candidateSha256: "b".repeat(64),
    fixtureId: "a".repeat(32),
    observationSha256: "e".repeat(64),
    sourceHeadSha256: "c".repeat(64),
  });
  const completionProjection = buildColimaLivePlanCompletionProjectionStructure({
    operationPlanSha256: liveProviderPlanDigest(
      liveProviderPlanBytes(operationPlan),
    ),
    planCloseSha256: "d".repeat(64),
    planSlotSha256: "f".repeat(64),
    preparationObservationSha256:
      operationPlan.preparation_observation_sha256,
  });
  return { completionProjection, operationPlan };
}

export function cleanEngineLiveEffectIntentFixture(
  completionProjection,
  operationPlan,
) {
  return buildColimaLiveEffectIntentCandidateStructure({
    completionProjection,
    operationPlan,
  });
}

export function cleanEngineLiveEmptyPreEffectPrefixFixture(intent) {
  return {
    entry_count: 0,
    intent_candidate_sha256: digest(intent),
    schema: COLIMA_LIVE_EMPTY_PRE_EFFECT_PREFIX_SCHEMA,
  };
}
