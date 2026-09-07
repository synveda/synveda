#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  colimaLiveLimaNetworkConfigBytesForTest,
} from "../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";
import {
  COLIMA_LIVE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
  COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  COLIMA_LIVE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
  COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  LiveProviderProcessStartFailure,
  authorizeColimaLiveProviderProcessStartEffect,
  buildColimaLiveCompletedProviderStartDecisionProjectionStructure,
  buildColimaLiveProviderProcessStartEffectCandidateStructure,
  buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
  liveProviderProcessStartBytes,
  liveProviderProcessStartDigest,
  validateColimaLiveCompletedProviderStartDecisionProjectionStructure,
  validateColimaLiveProviderProcessStartEffectCandidateStructure,
  validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
} from "../deploy/compose/scripts/clean-engine-live-provider-process-start.mjs";
import {
  cleanEngineLiveBaselineNetworkConfigBytesFixture,
  cleanEngineLiveProviderProcessStartFreshAdmissionFixture,
  cleanEngineLiveProviderProcessStartRootObservationFixture,
  cleanEngineLiveProviderProcessStartSourceFixture,
} from "./fixtures/clean-engine-live-provider-process-start-fixture.mjs";
import {
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
} from "../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";

function clone(value) {
  return structuredClone(value);
}

function digest(value) {
  return liveProviderProcessStartDigest(liveProviderProcessStartBytes(value));
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function candidateExpectation(fixture) {
  return {
    completedStartDecisionProjection:
      fixture.completedStartDecisionProjection,
    rootObservation: fixture.rootObservation,
    source: fixture.source,
  };
}

test("production and fixture post-decision admissions are exact and deny only", () => {
  assert.deepEqual(
    cleanEngineLiveBaselineNetworkConfigBytesFixture(),
    colimaLiveLimaNetworkConfigBytesForTest(),
  );
  const variants = [
    {
      admissionDigest:
        "5fc002ad29401b719342a66e509346aba2f6cd3656dc44e2f891d172eac2760c",
      candidateDigest:
        "218851027e4cf2b2cd027fd1d53a97905d87d01005ef813b9da627663b66b61a",
      evidenceClass: "production-pinned",
      fixture: cleanEngineLiveProviderProcessStartFreshAdmissionFixture(false),
      projectionDigest:
        "ee782e43c15d0243eece07a91c5592194eb408122f09fc91a059998b96c9eab4",
    },
    {
      admissionDigest:
        "d93445c2fd7c3aa5b2b725477564d4de75c9ff588ee2e2aa146a754895681314",
      candidateDigest:
        "a2125c69bc286c639202ddd0cdec122351e5aad88a68ff7cd5bb389ced6b4120",
      evidenceClass: "fixture-only",
      fixture: cleanEngineLiveProviderProcessStartFreshAdmissionFixture(true),
      projectionDigest:
        "3e4ba398a7e80aa201bb935013d701ff89f8872ee83af6956ebd9d258480db64",
    },
  ];
  assert.equal(
    COLIMA_LIVE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
    "synveda.clean-engine.colima-live-completed-provider-start-decision-projection.v1",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-completed-provider-start-decision-projection.v1",
  );
  assert.equal(
    COLIMA_LIVE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
    "synveda.clean-engine.colima-live-process-start-effect-candidate.v1",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-process-start-effect-candidate.v1",
  );
  assert.equal(
    COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
    "synveda.clean-engine.colima-live-process-start-effect-fresh-admission.v1",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-process-start-effect-fresh-admission.v1",
  );

  for (const value of variants) {
    const { admission, completedStartDecisionProjection, source } =
      value.fixture;
    const candidate = admission.process_start_effect_candidate;
    assert.notEqual(candidate, null);
    assert.equal(completedStartDecisionProjection.evidence_class, value.evidenceClass);
    assert.equal(candidate.evidence_class, value.evidenceClass);
    assert.equal(candidate.effect_name, "provider-process-start");
    assert.equal(
      candidate.effect_authorization,
      "denied-no-durable-effect-contract",
    );
    assert.equal(candidate.process_state, "not-reached");
    assert.equal(candidate.durable_effect_slot_required, true);
    assert.equal(candidate.future_effect_fresh_admission_required, true);
    assert.equal(candidate.cross_namespace_atomic_reservation, false);
    for (const field of [
      "cleanup_authorized",
      "effect_execution_authorized",
      "environment_publication_authorized",
      "evidence_publication_authorized",
      "finalization_authorized",
      "lifecycle_exposed",
      "process_group_ownership_authorized",
      "process_signal_authorized",
      "process_spawn_authorized",
      "process_start_authorized",
      "provider_adapter_execution_authorized",
      "provider_artifact_publication_authorized",
      "provider_recovery_authorized",
      "provider_root_mutation_authorized",
      "receipt_publication_authorized",
      "runtime_publication_authorized",
      "state_effect_slot_publication_authorized",
    ]) {
      assert.equal(candidate[field], false, field);
    }
    validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
      completedStartDecisionProjection,
      source,
    );
    validateColimaLiveProviderProcessStartEffectCandidateStructure(
      candidate,
      candidateExpectation(value.fixture),
    );
    validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
      admission,
      source,
    );
    assert.equal(digest(completedStartDecisionProjection), value.projectionDigest);
    assert.equal(digest(candidate), value.candidateDigest);
    assert.equal(digest(admission), value.admissionDigest);
    assertRecursivelyFrozen(completedStartDecisionProjection);
    assertRecursivelyFrozen(candidate);
    assertRecursivelyFrozen(admission);
  }
  assert.notEqual(variants[0].projectionDigest, variants[1].projectionDigest);
  assert.notEqual(variants[0].candidateDigest, variants[1].candidateDigest);
  assert.notEqual(variants[0].admissionDigest, variants[1].admissionDigest);
});

test("the completed decision projection refuses every authority binding drift", () => {
  const fixture = cleanEngineLiveProviderProcessStartFreshAdmissionFixture();
  const projection = fixture.completedStartDecisionProjection;
  const source = fixture.source;
  const hashFields = new Set([
    "completed_intent_projection_sha256",
    "decision_candidate_sha256",
    "provider_start_decision_close_sha256",
    "provider_start_decision_operation_contract_sha256",
    "provider_start_decision_publication_plan_sha256",
    "provider_start_decision_slot_sha256",
  ]);
  for (const field of Object.keys(projection)) {
    const changed = clone(projection);
    changed[field] = hashFields.has(field)
      ? "a".repeat(64)
      : `${String(changed[field])}-changed`;
    assert.throws(
      () =>
        validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
          changed,
          source,
        ),
      /completed start decision projection was refused/u,
      field,
    );
  }
  for (const invalidHash of ["0".repeat(64), "A".repeat(64), "a".repeat(63)]) {
    assert.throws(
      () =>
        validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
          { ...projection, provider_start_decision_slot_sha256: invalidHash },
          source,
        ),
      /completed start decision projection was refused/u,
    );
  }

  for (const changedSource of [
    { ...source, closeAuthority: "recovery" },
    { ...source, fixtureOnly: true },
    { ...source, extra: true },
    {
      ...source,
      startDecisionCompletion: {
        ...source.startDecisionCompletion,
        decision_candidate_sha256: "a".repeat(64),
      },
    },
    {
      ...source,
      startDecisionPublicationPlan: {
        ...source.startDecisionPublicationPlan,
        state_integration: "not-integrated",
      },
    },
  ]) {
    assert.throws(
      () =>
        validateColimaLiveCompletedProviderStartDecisionProjectionStructure(
          projection,
          changedSource,
        ),
      /process start source/u,
    );
  }
});

test("fresh pristine namespaces yield only a false-authority candidate", () => {
  for (const fixtureOnly of [false, true]) {
    const fixture =
      cleanEngineLiveProviderProcessStartFreshAdmissionFixture(fixtureOnly);
    const expected = candidateExpectation(fixture);
    const candidate = fixture.admission.process_start_effect_candidate;
    const changedRootObservation = clone(fixture.rootObservation);
    const limaRoot = changedRootObservation.root_observations.find(
      (root) => root.role === "lima-home-namespace",
    );
    limaRoot.baseline_descriptors[0].descriptor_sha256 = "0".repeat(64);
    assert.throws(
      () =>
        buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
          completedStartDecisionProjection:
            fixture.completedStartDecisionProjection,
          rootObservation: changedRootObservation,
          source: fixture.source,
        }),
      /baseline descriptor was refused/u,
    );
    const falseFields = Object.keys(candidate).filter((field) =>
      field.endsWith("_authorized") || field === "lifecycle_exposed",
    );
    for (const field of falseFields) {
      const changed = { ...candidate, [field]: true };
      assert.throws(
        () =>
          validateColimaLiveProviderProcessStartEffectCandidateStructure(
            changed,
            expected,
          ),
        /process start candidate was refused/u,
        field,
      );
    }
    for (const field of [
      "durable_effect_slot_required",
      "future_effect_fresh_admission_required",
    ]) {
      assert.throws(
        () =>
          validateColimaLiveProviderProcessStartEffectCandidateStructure(
            { ...candidate, [field]: false },
            expected,
          ),
        /process start candidate was refused/u,
        field,
      );
    }

    for (const role of COLIMA_LIVE_MUTATION_SURFACE_ROLES) {
      const rootObservation =
        cleanEngineLiveProviderProcessStartRootObservationFixture(
          fixture.source,
          role,
        );
      const collision =
        buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
          completedStartDecisionProjection:
            fixture.completedStartDecisionProjection,
          rootObservation,
          source: fixture.source,
        });
      assert.equal(collision.process_start_effect_candidate, null);
      assert.equal(
        collision.root_observation.root_set_disposition,
        "foreign-collision",
      );
      assert.throws(
        () =>
          buildColimaLiveProviderProcessStartEffectCandidateStructure({
            completedStartDecisionProjection:
              fixture.completedStartDecisionProjection,
            rootObservation,
            source: fixture.source,
          }),
        /collision cannot become a process start candidate/u,
      );
      assert.throws(
        () =>
          validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
            {
              ...collision,
              process_start_effect_candidate: candidate,
            },
            fixture.source,
          ),
        /collision admitted a process start effect/u,
      );
    }
    assert.throws(
      () =>
        validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
          { ...fixture.admission, process_start_effect_candidate: null },
          fixture.source,
        ),
      /process start candidate/u,
    );
  }
});

test("builders and the deliberate authorization denial are closed", () => {
  const fixture = cleanEngineLiveProviderProcessStartFreshAdmissionFixture();
  const input = candidateExpectation(fixture);
  for (const builder of [
    () =>
      buildColimaLiveCompletedProviderStartDecisionProjectionStructure({
        ...fixture.source,
        extra: true,
      }),
    () =>
      buildColimaLiveProviderProcessStartEffectCandidateStructure({
        ...input,
        extra: true,
      }),
    () =>
      buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
        ...input,
        extra: true,
      }),
  ]) {
    assert.throws(builder, /fields were refused/u);
  }
  assert.throws(
    () =>
      validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
        { ...fixture.admission, authority: "effect-authority" },
        fixture.source,
      ),
    /process start admission was refused/u,
  );
  assert.throws(
    () =>
      authorizeColimaLiveProviderProcessStartEffect({
        admission: fixture.admission,
        source: fixture.source,
      }),
    (error) =>
      error instanceof LiveProviderProcessStartFailure &&
      error.exitStatus === 69 &&
      error.message ===
        "Colima live provider process start remains disabled pending durable effect authority",
  );
  const fixtureVariant =
    cleanEngineLiveProviderProcessStartFreshAdmissionFixture(true);
  assert.throws(
    () =>
      authorizeColimaLiveProviderProcessStartEffect({
        admission: fixtureVariant.admission,
        source: fixtureVariant.source,
      }),
    /fixture process start admission cannot authorize an effect/u,
  );
  assert.throws(
    () =>
      authorizeColimaLiveProviderProcessStartEffect({
        admission: fixture.admission,
        source: fixture.source,
        extra: true,
      }),
    /authorization arguments fields were refused/u,
  );
});

test("hidden inherited symbol and accessor payloads are refused", () => {
  const fixture = cleanEngineLiveProviderProcessStartFreshAdmissionFixture();
  const hiddenRoot = clone(fixture.rootObservation);
  Object.defineProperty(hiddenRoot, "command", {
    enumerable: false,
    value: { argv: ["unexpected"] },
  });
  const symbolRoot = clone(fixture.rootObservation);
  symbolRoot[Symbol("command")] = { argv: ["unexpected"] };
  const inheritedRoot = Object.assign(
    Object.create({ command: { argv: ["unexpected"] } }),
    clone(fixture.rootObservation),
  );
  const inheritedArrayRoot = clone(fixture.rootObservation);
  Object.setPrototypeOf(
    inheritedArrayRoot.root_observations,
    Object.create(Array.prototype, {
      command: {
        enumerable: true,
        value: { argv: ["unexpected"] },
      },
    }),
  );
  let getterRead = false;
  const accessorRoot = clone(fixture.rootObservation);
  Object.defineProperty(accessorRoot, "command", {
    enumerable: true,
    get() {
      getterRead = true;
      return { argv: ["unexpected"] };
    },
  });
  for (const rootObservation of [
    hiddenRoot,
    symbolRoot,
    inheritedRoot,
    inheritedArrayRoot,
    accessorRoot,
  ]) {
    assert.throws(
      () =>
        buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
          completedStartDecisionProjection:
            fixture.completedStartDecisionProjection,
          rootObservation,
          source: fixture.source,
        }),
      /(?:symbol property|plain data-object prototype|hidden or accessor property|not a dense data array)/u,
    );
  }
  assert.equal(getterRead, false);

  const hiddenSource = clone(fixture.source);
  Object.defineProperty(hiddenSource.startDecisionSource, "environment", {
    enumerable: false,
    value: { secret: "unexpected" },
  });
  assert.throws(
    () =>
      buildColimaLiveCompletedProviderStartDecisionProjectionStructure(
        hiddenSource,
      ),
    /hidden or accessor property/u,
  );

  const shallowFrozenRoot = clone(fixture.rootObservation);
  Object.freeze(shallowFrozenRoot);
  const admission =
    buildColimaLiveProviderProcessStartEffectFreshAdmissionStructure({
      completedStartDecisionProjection:
        fixture.completedStartDecisionProjection,
      rootObservation: shallowFrozenRoot,
      source: fixture.source,
    });
  assertRecursivelyFrozen(admission);
});

test("serialized admissions contain no process identity or private execution input", () => {
  for (const fixtureOnly of [false, true]) {
    const fixture =
      cleanEngineLiveProviderProcessStartFreshAdmissionFixture(fixtureOnly);
    const raw = liveProviderProcessStartBytes(fixture.admission).toString("utf8");
    assert.doesNotMatch(raw, /(?:\/Users\/|\/home\/|binding_key|credential|secret)/iu);
    assert.doesNotMatch(raw, /"(?:command|environment|home|path|pid|pgid|socket)"\s*:/iu);
    assert.doesNotMatch(raw, /(?:hostagent|guest-engine|docker-context|provider-identity)/iu);
  }
});

test("the post-decision boundary owns no process, mutation or lifecycle seam", () => {
  const source = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-live-provider-process-start.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(
    source,
    /node:(?:child_process|dgram|fs|http|https|net|tls|worker_threads)/u,
  );
  assert.doesNotMatch(
    source,
    /clean-engine-(?:receipts|state|provider-process-contract)|\/fixtures\//u,
  );
  assert.doesNotMatch(
    source,
    /export function (?:cleanup|execute|finalize|launch|publish|recover|settle|signal|spawn)/u,
  );
  assert.doesNotMatch(
    source,
    /\b(?:exec|fork|kill|link|mkdir|rename|rmdir|spawn|unlink|writeFile)[A-Za-z0-9_]*\s*\(/u,
  );
  assert.doesNotMatch(source, /mutation-(?:journal|slot|close|recovery)/u);
  assert.doesNotMatch(source, /PROCESS_START_EFFECT_OPERATION_(?:KIND|CONTRACT)/u);

  for (const path of [
    "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
    "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
    "../deploy/compose/scripts/clean-engine-receipts.mjs",
  ]) {
    const consumer = readFileSync(new URL(path, import.meta.url), "utf8");
    assert.doesNotMatch(consumer, /clean-engine-live-provider-process-start/u);
  }
  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /process-start-effect/u);
  const processContract = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(processContract, /\bCOLIMA_LIVE_[A-Z0-9_]+\b/u);
  assert.doesNotMatch(
    processContract,
    /\b(?:authorize|validate)ColimaLive[A-Za-z0-9_]*\b/u,
  );
  assert.doesNotMatch(processContract, /synveda\.clean-engine\.colima-live-/u);
});
