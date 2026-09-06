import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT,
  COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_RESERVATION_ACTION,
  COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
  COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT,
  COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
  LiveProviderReservationFailure,
  authorizeColimaLiveProviderReservationEffect,
  buildColimaLiveProviderReservationCompletion,
  buildColimaLiveProviderReservationPublicationPlan,
  buildColimaLiveProviderReservationSettlement,
  buildColimaLiveProviderReservationWitness,
  liveProviderReservationBytes,
  liveProviderReservationDigest,
  validateColimaLiveProviderReservationCompletion,
  validateColimaLiveProviderReservationPublicationPlan,
  validateColimaLiveProviderReservationSettlement,
  validateColimaLiveProviderReservationWitness,
} from "../deploy/compose/scripts/clean-engine-live-provider-reservation.mjs";
import { buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure } from "../deploy/compose/scripts/clean-engine-live-provider-process-start.mjs";
import { cleanEngineLiveProviderReservationFixture } from "./fixtures/clean-engine-live-provider-reservation-fixture.mjs";

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function rebuildPlan(base, overrides = {}) {
  return buildColimaLiveProviderReservationPublicationPlan({
    admission: base.admission,
    namespaceBindings: base.namespaceBindings,
    providerRootIdentity: base.publicationPlan.provider_root_identity,
    source: base.source,
    stateRunIdentity: base.publicationPlan.state_run_identity,
    ...overrides,
  });
}

test("reservation contracts are distinct, stable and process-inert", () => {
  assert.equal(COLIMA_LIVE_PROVIDER_RESERVATION_ACTION, "provider-reservation");
  assert.equal(
    COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    ".synveda-clean-engine-provider-reservation",
  );
  assert.notEqual(
    COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
    COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
  );
  for (const [contract, digest, pinnedDigest] of [
    [
      COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT,
      COLIMA_LIVE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
      "eda9f544d1fb9752debcff6bb486081d855a9e0cead01e42d78979da18674c98",
    ],
    [
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT,
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_OPERATION_CONTRACT_SHA256,
      "3c8da820554ef086ed0a84144512b12c845412db682be07064b7e7154fbdc23b",
    ],
  ]) {
    assert.equal(digest, pinnedDigest);
    assert.equal(
      liveProviderReservationDigest(liveProviderReservationBytes(contract)),
      digest,
    );
    assert.equal(contract.capabilities.marker_link_authorized, true);
    assert.equal(contract.capabilities.marker_retirement_authorized, true);
    assert.equal(contract.capabilities.state_evidence_publication_authorized, true);
    assert.equal(contract.capabilities.reservation_recovery_authorized, true);
    for (const key of [
      "process_start_authorized",
      "process_spawn_authorized",
      "process_signal_authorized",
      "process_group_ownership_authorized",
      "provider_adapter_execution_authorized",
      "general_provider_root_mutation_authorized",
      "provider_artifact_publication_authorized",
      "evidence_publication_authorized",
      "receipt_publication_authorized",
      "environment_publication_authorized",
      "runtime_publication_authorized",
      "provider_recovery_authorized",
      "cleanup_authorized",
      "lifecycle_exposed",
      "finalization_authorized",
    ]) {
      assert.equal(contract.capabilities[key], false, key);
    }
    assertRecursivelyFrozen(contract);
  }
});

test("production and fixture reservation evidence is closed and class-separated", () => {
  for (const fixtureOnly of [false, true]) {
    const value = cleanEngineLiveProviderReservationFixture(fixtureOnly);
    assert.equal(
      validateColimaLiveProviderReservationPublicationPlan(
        value.publicationPlan,
        value.source,
      ),
      value.publicationPlan,
    );
    assert.equal(
      validateColimaLiveProviderReservationWitness(value.witness, {
        publicationPlan: value.publicationPlan,
        source: value.source,
      }),
      value.witness,
    );
    assert.equal(
      validateColimaLiveProviderReservationSettlement(value.settlement, {
        publicationPlan: value.publicationPlan,
        slotSequence: value.slotSequence,
        slotSha256: value.slotSha256,
        source: value.source,
        witness: value.witness,
      }),
      value.settlement,
    );
    assert.equal(
      validateColimaLiveProviderReservationCompletion(value.completion, {
        closeSha256: value.completion.close_sha256,
        publicationPlan: value.publicationPlan,
        settlement: value.settlement,
        slotSequence: value.slotSequence,
        slotSha256: value.slotSha256,
        source: value.source,
        witness: value.witness,
      }),
      value.completion,
    );
    for (const artifact of [
      value.publicationPlan,
      value.witness,
      value.settlement,
      value.completion,
    ]) {
      assertRecursivelyFrozen(artifact);
      assert.doesNotMatch(
        liveProviderReservationBytes(artifact).toString("utf8"),
        /(?:\/tmp\/|provider_root|state_run).*path/u,
      );
    }
    assert.equal(value.witness.process_attempt, "not-started");
    assert.equal(
      value.settlement.reservation_disposition,
      "retirement-authorized-without-process",
    );
    assert.equal(value.settlement.process_start_authorized, false);
  }

  const production = cleanEngineLiveProviderReservationFixture(false);
  const fixture = cleanEngineLiveProviderReservationFixture(true);
  assert.throws(
    () =>
      validateColimaLiveProviderReservationPublicationPlan(
        production.publicationPlan,
        fixture.source,
      ),
    LiveProviderReservationFailure,
  );
  assert.notEqual(
    production.publicationPlan.operation_contract_sha256,
    fixture.publicationPlan.operation_contract_sha256,
  );
});

test("reservation planning requires one pristine inert admission and one device", () => {
  const base = cleanEngineLiveProviderReservationFixture(false);
  const collisionRoot = {
    ...base.rootObservation,
    root_observations: base.rootObservation.root_observations.map(
      (root, index) =>
        index === 0 ? { ...root, disposition: "foreign-collision" } : root,
    ),
    root_set_disposition: "foreign-collision",
  };
  const collisionAdmission =
    buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
      completedStartDecisionProjection: base.completedStartDecisionProjection,
      rootObservation: collisionRoot,
      source: base.source,
    });
  assert.equal(collisionAdmission.process_start_effect_candidate, null);
  assert.throws(
    () => rebuildPlan(base, { admission: collisionAdmission }),
    /not pristine and inert/u,
  );

  assert.throws(
    () =>
      rebuildPlan(base, {
        stateRunIdentity: {
          ...base.publicationPlan.state_run_identity,
          device: "72",
        },
      }),
    /publication plan/u,
  );
  assert.throws(
    () =>
      rebuildPlan(base, {
        stateRunIdentity: {
          ...base.publicationPlan.provider_root_identity,
        },
      }),
    /publication plan/u,
  );
  assert.throws(
    () =>
      rebuildPlan(base, {
        namespaceBindings: base.namespaceBindings.map((binding, index) =>
          index === 1
            ? {
                ...binding,
                inode: base.namespaceBindings[0].inode,
              }
            : binding,
        ),
      }),
    /publication plan/u,
  );
  assert.throws(
    () =>
      rebuildPlan(base, {
        namespaceBindings: base.namespaceBindings.map((binding, index) =>
          index === 2 ? { ...binding, device: "72" } : binding,
        ),
      }),
    /namespace binding/u,
  );
  for (const key of ["device", "inode", "mode", "uid"]) {
    const changed = {
      ...base.publicationPlan.provider_root_identity,
      [key]: key === "mode" ? "0755" : "not-decimal",
    };
    assert.throws(
      () => rebuildPlan(base, { providerRootIdentity: changed }),
      /provider-root identity/u,
      key,
    );
  }
});

test("reservation artifacts reject semantic tamper even when reserialized", () => {
  const base = cleanEngineLiveProviderReservationFixture(false);
  const planMutants = [
    { ...clone(base.publicationPlan), fixed_marker_name: ".different" },
    { ...clone(base.publicationPlan), reservation_authorization: "authorized" },
    { ...clone(base.publicationPlan), operation_contract_sha256: "0".repeat(64) },
    { ...clone(base.publicationPlan), state_integration: "mutation-journal-v5" },
  ];
  for (const mutant of planMutants) {
    assert.throws(
      () =>
        validateColimaLiveProviderReservationPublicationPlan(
          mutant,
          base.source,
        ),
      LiveProviderReservationFailure,
    );
  }

  for (const mutant of [
    { ...clone(base.witness), marker_link_count: 1 },
    { ...clone(base.witness), process_attempt: "started" },
    { ...clone(base.witness), reservation_disposition: "not-reached" },
  ]) {
    assert.throws(
      () =>
        validateColimaLiveProviderReservationWitness(mutant, {
          publicationPlan: base.publicationPlan,
          source: base.source,
        }),
      LiveProviderReservationFailure,
    );
  }

  for (const mutant of [
    { ...clone(base.settlement), authority: "caller" },
    { ...clone(base.settlement), process_start_authorized: true },
    { ...clone(base.settlement), marker_retirement: "not-checked" },
    { ...clone(base.settlement), reservation_disposition: "aborted-before-effect" },
    { ...clone(base.settlement), reservation_witness_sha256: "8".repeat(64) },
  ]) {
    assert.throws(
      () =>
        validateColimaLiveProviderReservationSettlement(mutant, {
          publicationPlan: base.publicationPlan,
          slotSequence: base.slotSequence,
          slotSha256: base.slotSha256,
          source: base.source,
          witness: base.witness,
        }),
      LiveProviderReservationFailure,
    );
  }
  const changedObservation = {
    ...clone(base.rootObservation),
    root_set_disposition: "foreign-collision",
  };
  assert.throws(
    () =>
      buildColimaLiveProviderReservationSettlement({
        authority: "owner",
        authoritySha256: base.slotSha256,
        preRetirementRootObservation: changedObservation,
        publicationPlan: base.publicationPlan,
        slotSequence: base.slotSequence,
        slotSha256: base.slotSha256,
        source: base.source,
        witness: base.witness,
      }),
    /pre-retirement observation/u,
  );
});

test("caller-held reservation data never authorizes an effect", () => {
  const base = cleanEngineLiveProviderReservationFixture(false);
  assert.throws(
    () =>
      authorizeColimaLiveProviderReservationEffect({
        publicationPlan: base.publicationPlan,
        source: base.source,
      }),
    /cannot authorize an effect/u,
  );
});

test("reservation module has no process, adapter or general mutation surface", () => {
  const source = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-live-provider-reservation.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(source, /node:child_process/u);
  assert.doesNotMatch(source, /node:fs/u);
  assert.doesNotMatch(
    source,
    /(?:spawn|exec|kill|signal|launchControlled|authorizeProviderAdapterPlanning)\s*\(/u,
  );
  assert.doesNotMatch(source, /clean-engine-provider-adapter-registry/u);
});

test("builders reject malformed slots and stale close identities", () => {
  const base = cleanEngineLiveProviderReservationFixture(false);
  for (const slotSequence of [-1, 64, 1.5]) {
    assert.throws(
      () =>
        buildColimaLiveProviderReservationWitness({
          publicationPlan: base.publicationPlan,
          slotSequence,
          slotSha256: base.slotSha256,
          source: base.source,
        }),
      /slot binding/u,
    );
  }
  assert.throws(
    () =>
      buildColimaLiveProviderReservationCompletion({
        closeSha256: "0".repeat(64),
        publicationPlan: base.publicationPlan,
        settlement: base.settlement,
        slotSequence: base.slotSequence,
        slotSha256: base.slotSha256,
        source: base.source,
        witness: base.witness,
      }),
    /completion/u,
  );

  const accessor = clone(base.publicationPlan);
  Object.defineProperty(accessor, "fixture_id", {
    enumerable: true,
    get() {
      return base.publicationPlan.fixture_id;
    },
  });
  assert.throws(
    () =>
      validateColimaLiveProviderReservationPublicationPlan(
        accessor,
        base.source,
      ),
    /accessor/u,
  );
});
