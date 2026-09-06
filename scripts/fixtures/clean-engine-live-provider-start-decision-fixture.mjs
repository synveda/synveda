import { createHash } from "node:crypto";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
  COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
  buildColimaLiveEmptyPreEffectPrefixStructure,
  buildColimaLiveProviderIntentCompletion,
  buildColimaLiveProviderIntentPublicationPlan,
  liveProviderIntentBytes,
  liveProviderIntentDigest,
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

const ZERO_SHA256 = "0".repeat(64);
const LIMA_NETWORK_CONFIG_BYTES = Buffer.from(
  "networks:\n" +
    "  user-v2:\n" +
    "    mode: user-v2\n" +
    "    gateway: 192.168.5.2\n" +
    "    netmask: 255.255.255.0\n",
  "utf8",
);

export function cleanEngineLiveBaselineNetworkConfigBytesFixture() {
  return Buffer.from(LIMA_NETWORK_CONFIG_BYTES);
}

function digest(value) {
  return liveProviderIntentDigest(liveProviderIntentBytes(value));
}

function rawSha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function baselineDescriptorBindings(namespaceIdentityHmacSha256, role) {
  if (role !== "lima-home-namespace") return [];
  const directoryIdentity = "e".repeat(64);
  const fileIdentity = "f".repeat(64);
  const common = {
    device: "1",
    links: "1",
    uid: "501",
  };
  const descriptors = [
    {
      content_disposition: "metadata-only",
      content_sha256: ZERO_SHA256,
      ctime_nanoseconds: "1000000200",
      depth: 1,
      ...common,
      directory_entry_count: 1,
      inode: "900",
      mode: "0700",
      mtime_nanoseconds: "1000000200",
      name_bytes: Buffer.byteLength("_config", "utf8"),
      parent_identity_hmac_sha256: namespaceIdentityHmacSha256,
      relative_identity_hmac_sha256: directoryIdentity,
      size: "0",
      symlink_target_sha256: ZERO_SHA256,
      type: "directory",
    },
    {
      content_disposition: "hashed",
      content_sha256: rawSha256(LIMA_NETWORK_CONFIG_BYTES),
      ctime_nanoseconds: "1000000300",
      depth: 2,
      ...common,
      directory_entry_count: 0,
      inode: "901",
      mode: "0600",
      mtime_nanoseconds: "1000000300",
      name_bytes: Buffer.byteLength("networks.yaml", "utf8"),
      parent_identity_hmac_sha256: directoryIdentity,
      relative_identity_hmac_sha256: fileIdentity,
      size: String(LIMA_NETWORK_CONFIG_BYTES.length),
      symlink_target_sha256: ZERO_SHA256,
      type: "regular",
    },
  ];
  return descriptors.map((descriptor) => ({
    descriptor_sha256: digest(descriptor),
    relative_identity_hmac_sha256:
      descriptor.relative_identity_hmac_sha256,
  }));
}

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
    const namespaceIdentityHmacSha256 = "13579b"[index].repeat(64);
    const baselineRelativeIdentityHmacSha256 =
      role === "lima-home-namespace"
        ? ["e".repeat(64), "f".repeat(64)]
        : [];
    return {
      baseline_descriptor_set_hmac_sha256: "abcdef"[index].repeat(64),
      baseline_descriptors: baselineDescriptorBindings(
        namespaceIdentityHmacSha256,
        role,
      ),
      baseline_relative_identity_hmac_sha256:
        baselineRelativeIdentityHmacSha256,
      disposition: collision ? "foreign-collision" : "observed-pristine",
      namespace_identity_hmac_sha256: namespaceIdentityHmacSha256,
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
