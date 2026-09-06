import {
  buildColimaLiveProviderReservationCompletion,
  buildColimaLiveProviderReservationPublicationPlan,
  buildColimaLiveProviderReservationSettlement,
  buildColimaLiveProviderReservationWitness,
} from "../../deploy/compose/scripts/clean-engine-live-provider-reservation.mjs";
import { cleanEngineLiveProviderProcessStartFreshAdmissionFixture } from "./clean-engine-live-provider-process-start-fixture.mjs";

export function cleanEngineLiveProviderReservationFixture(
  fixtureOnly = false,
) {
  const upstream =
    cleanEngineLiveProviderProcessStartFreshAdmissionFixture(fixtureOnly);
  const common = {
    device: "71",
    mode: "0700",
    uid: "501",
  };
  const namespaceBindings = upstream.rootObservation.root_observations.map(
    (root, index) => ({
      ...common,
      inode: String(400 + index),
      namespace_identity_hmac_sha256:
        root.namespace_identity_hmac_sha256,
      observed_entry_set_hmac_sha256:
        root.observed_entry_set_hmac_sha256,
      role: root.role,
    }),
  );
  const publicationPlan =
    buildColimaLiveProviderReservationPublicationPlan({
      admission: upstream.admission,
      namespaceBindings,
      providerRootIdentity: {
        ...common,
        inode: "300",
      },
      source: upstream.source,
      stateRunIdentity: {
        ...common,
        inode: "200",
      },
    });
  const slotSequence = 3;
  const slotSha256 = "6".repeat(64);
  const witness = buildColimaLiveProviderReservationWitness({
    publicationPlan,
    slotSequence,
    slotSha256,
    source: upstream.source,
  });
  const settlement = buildColimaLiveProviderReservationSettlement({
    authority: "owner",
    authoritySha256: slotSha256,
    preRetirementRootObservation: upstream.rootObservation,
    publicationPlan,
    slotSequence,
    slotSha256,
    source: upstream.source,
    witness,
  });
  const completion = buildColimaLiveProviderReservationCompletion({
    closeSha256: "7".repeat(64),
    publicationPlan,
    settlement,
    slotSequence,
    slotSha256,
    source: upstream.source,
    witness,
  });
  return {
    ...upstream,
    completion,
    namespaceBindings,
    publicationPlan,
    settlement,
    slotSequence,
    slotSha256,
    witness,
  };
}
