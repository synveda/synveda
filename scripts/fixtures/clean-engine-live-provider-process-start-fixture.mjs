import {
  buildColimaLiveProviderStartDecisionCompletion,
  buildColimaLiveProviderStartDecisionPublicationPlan,
} from "../../deploy/compose/scripts/clean-engine-live-provider-start-decision.mjs";
import {
  buildColimaLiveCompletedProviderStartDecisionProjectionStructure,
  buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
} from "../../deploy/compose/scripts/clean-engine-live-provider-process-start.mjs";
import {
  cleanEngineLiveProviderStartFreshAdmissionFixture,
  cleanEngineLiveProviderStartFreshRootObservationFixture,
} from "./clean-engine-live-provider-start-decision-fixture.mjs";

export function cleanEngineLiveProviderProcessStartSourceFixture(
  fixtureOnly = false,
) {
  const upstream =
    cleanEngineLiveProviderStartFreshAdmissionFixture(fixtureOnly);
  const startDecisionPublicationPlan =
    buildColimaLiveProviderStartDecisionPublicationPlan({
      admission: upstream.admission,
      source: upstream.source,
    });
  const startDecisionCompletion =
    buildColimaLiveProviderStartDecisionCompletion({
      closeSha256: "4".repeat(64),
      publicationPlan: startDecisionPublicationPlan,
      slotSha256: "5".repeat(64),
      source: upstream.source,
    });
  const source = {
    closeAuthority: "owner",
    fixtureOnly,
    startDecisionCompletion,
    startDecisionPublicationPlan,
    startDecisionSource: upstream.source,
  };
  const completedStartDecisionProjection =
    buildColimaLiveCompletedProviderStartDecisionProjectionStructure(source);
  return {
    completedStartDecisionProjection,
    operationPlan: upstream.operationPlan,
    source,
  };
}

export function cleanEngineLiveProviderProcessStartRootObservationFixture(
  source,
  collisionRole,
) {
  return cleanEngineLiveProviderStartFreshRootObservationFixture(
    source.startDecisionSource,
    collisionRole,
  );
}

export function cleanEngineLiveProviderProcessStartFreshAdmissionFixture(
  fixtureOnly = false,
) {
  const { completedStartDecisionProjection, operationPlan, source } =
    cleanEngineLiveProviderProcessStartSourceFixture(fixtureOnly);
  const rootObservation =
    cleanEngineLiveProviderProcessStartRootObservationFixture(source);
  const admission =
    buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
      completedStartDecisionProjection,
      rootObservation,
      source,
    });
  return {
    admission,
    completedStartDecisionProjection,
    operationPlan,
    rootObservation,
    source,
  };
}
