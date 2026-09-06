import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
  COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
  buildColimaLiveEmptyPreEffectPrefixStructure,
  buildColimaLiveProviderIntentCompletion,
  buildColimaLiveProviderIntentPublicationPlan,
} from "../../deploy/compose/scripts/clean-engine-live-provider-intent.mjs";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
} from "../../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";
import {
  buildColimaLiveCompletedProviderIntentProjectionStructure,
  buildColimaLiveProviderStartFreshAdmissionStructure,
} from "../../deploy/compose/scripts/clean-engine-live-provider-start-decision.mjs";
import {
  cleanEngineLiveEffectIntentFixture,
  cleanEngineLivePlanCompletionProjectionFixture,
} from "./clean-engine-live-provider-intent-fixture.mjs";

function rootObservation(operationPlan, fixtureOnly, collisionRole) {
  const roles = COLIMA_LIVE_MUTATION_SURFACE_ROLES;
  const collisionRoles = new Set(
    Array.isArray(collisionRole)
      ? collisionRole
      : collisionRole === undefined
        ? []
        : [collisionRole],
  );
  const roots = roles.map((role, index) => {
    const collision = collisionRoles.has(role);
    return {
      disposition: collision ? "foreign-collision" : "observed-pristine",
      namespace_identity_hmac_sha256: "13579b"[index].repeat(64),
      observed_entry_set_hmac_sha256: (collision ? "bcdef1" : "2468ac")[
        index
      ].repeat(64),
      role,
    };
  });
  return {
    evidence_class: fixtureOnly ? "fixture-only" : "production-pinned",
    planned_names: {
      lima_instance: `colima-${operationPlan.provider_profile}`,
      provider_profile: operationPlan.provider_profile,
    },
    preparation_observation_sha256:
      operationPlan.preparation_observation_sha256,
    requirements_sha256: fixtureOnly
      ? "9".repeat(64)
      : operationPlan.requirements_sha256,
    root_observations: roots,
    root_set_disposition:
      collisionRoles.size === 0 ? "observed-pristine" : "foreign-collision",
    schema: fixtureOnly
      ? COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA
      : COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  };
}

export function cleanEngineLiveProviderStartDecisionSourceFixture(
  fixtureOnly = false,
) {
  const { completionProjection, operationPlan } =
    cleanEngineLivePlanCompletionProjectionFixture();
  const intent = cleanEngineLiveEffectIntentFixture(
    completionProjection,
    operationPlan,
  );
  const preEffectPrefix = buildColimaLiveEmptyPreEffectPrefixStructure({
    completionProjection,
    intent,
    operationPlan,
  });
  const intentAdmission = {
    authority: fixtureOnly
      ? "fixture-only-point-in-time-not-effect-authority"
      : "point-in-time-not-effect-authority",
    completion_projection: completionProjection,
    intent_candidate: intent,
    pre_effect_prefix: preEffectPrefix,
    root_observation: rootObservation(operationPlan, fixtureOnly),
    schema: fixtureOnly
      ? COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA
      : COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
    supervisor_process_group_label:
      `sv-c45-colima-pg-${operationPlan.fixture_id}-` +
      completionProjection.plan_slot_sha256.slice(0, 12),
  };
  const intentPublicationPlan = buildColimaLiveProviderIntentPublicationPlan({
    admission: intentAdmission,
    fixtureOnly,
    operationPlan,
  });
  const intentCompletion = buildColimaLiveProviderIntentCompletion({
    closeSha256: "6".repeat(64),
    fixtureOnly,
    publicationPlan: intentPublicationPlan,
    slotSha256: "7".repeat(64),
  });
  const source = {
    closeAuthority: "owner",
    fixtureOnly,
    intentCompletion,
    intentPublicationPlan,
  };
  const completedIntentProjection =
    buildColimaLiveCompletedProviderIntentProjectionStructure(source);
  return {
    completedIntentProjection,
    operationPlan,
    source,
  };
}

export function cleanEngineLiveProviderStartFreshRootObservationFixture(
  source,
  collisionRole,
) {
  return rootObservation(
    source.intentPublicationPlan.provider_operation_plan,
    source.fixtureOnly,
    collisionRole,
  );
}

export function cleanEngineLiveProviderStartFreshAdmissionFixture(
  fixtureOnly = false,
) {
  const { completedIntentProjection, operationPlan, source } =
    cleanEngineLiveProviderStartDecisionSourceFixture(fixtureOnly);
  const rootObservationValue =
    cleanEngineLiveProviderStartFreshRootObservationFixture(source);
  const admission = buildColimaLiveProviderStartFreshAdmissionStructure({
    completedIntentProjection,
    rootObservation: rootObservationValue,
    source,
  });
  return {
    admission,
    completedIntentProjection,
    operationPlan,
    rootObservation: rootObservationValue,
    source,
  };
}
