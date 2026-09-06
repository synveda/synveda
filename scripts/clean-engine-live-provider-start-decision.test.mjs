import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
  COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,
  COLIMA_LIVE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
  LiveProviderStartDecisionFailure,
  buildColimaLiveCompletedProviderIntentProjectionStructure,
  buildColimaLiveProviderStartDecisionCandidateStructure,
  buildColimaLiveProviderStartDecisionCompletion,
  buildColimaLiveProviderStartDecisionPublicationPlan,
  buildColimaLiveProviderStartFreshAdmissionStructure,
  liveProviderStartDecisionBytes,
  liveProviderStartDecisionDigest,
  validateColimaLiveCompletedProviderIntentProjectionStructure,
  validateColimaLiveProviderStartDecisionCandidateStructure,
  validateColimaLiveProviderStartDecisionCompletion,
  validateColimaLiveProviderStartDecisionPublicationPlan,
  validateColimaLiveProviderStartFreshAdmissionStructure,
} from "../deploy/compose/scripts/clean-engine-live-provider-start-decision.mjs";
import {
  authorizeProviderAdapter,
  authorizeProviderAdapterPlanning,
  ProviderAdapterRegistryFailure,
  resolveProviderAdapter,
} from "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs";
import {
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
} from "../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";
import {
  COLIMA_LIVE_PREPARATION_CONTRACT,
  COLIMA_LIVE_PREPARATION_CONTRACT_SHA256,
  CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
  ProviderProcessContractFailure,
  authorizeColimaLiveStart,
} from "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs";
import {
  cleanEngineLiveProviderStartDecisionSourceFixture,
  cleanEngineLiveProviderStartFreshAdmissionFixture,
  cleanEngineLiveProviderStartFreshRootObservationFixture,
} from "./fixtures/clean-engine-live-provider-start-decision-fixture.mjs";

function digest(value) {
  return liveProviderStartDecisionDigest(liveProviderStartDecisionBytes(value));
}

function expectRefusal(operation, exitStatus) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof LiveProviderStartDecisionFailure);
    if (exitStatus !== undefined) assert.equal(error.exitStatus, exitStatus);
    return true;
  });
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function candidateExpectation(fixture) {
  return {
    completedIntentProjection: fixture.completedIntentProjection,
    rootObservation: fixture.rootObservation,
    source: fixture.source,
  };
}

test("production and fixture process-start contracts are exact, inert and unregistered", () => {
  const variants = [
    {
      contract: COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT,
      digest: COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
      expectedDigest:
        "9b4957d5b0918b78adde1ce336c923499a205043f05845931456b0246c6fc6e6",
      evidenceClass: "production-pinned",
      fixtureOnly: false,
      intentCompletionSchema:
        "synveda.clean-engine.colima-live-provider-intent-completion.v1",
      intentContractSha256:
        "a89e57ff3769616ff5c88aa63de9dd854bd2b821cf7516db33fa0f09b775edfc",
      intentKind: "colima-live-provider-intent-publication-v1",
      kind: COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
    },
    {
      contract:
        COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT,
      digest:
        COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
      expectedDigest:
        "407c7b466548373c320d600344d6c488578afed150914b2b06cd47bf52d55167",
      evidenceClass: "fixture-only",
      fixtureOnly: true,
      intentCompletionSchema:
        "synveda.clean-engine.colima-live-fixture-provider-intent-completion.v1",
      intentContractSha256:
        "4963be65ad92040a20de26c4d048ee11858538c72820c657643ebeedb68f7400",
      intentKind: "colima-live-fixture-provider-intent-publication-v1",
      kind: COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
    },
  ];
  assert.equal(COLIMA_LIVE_PROVIDER_START_DECISION_ACTION, "provider-start-decision");
  assert.equal(
    COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
    "synveda.clean-engine.colima-live-provider-start-decision-publication-plan.v2",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-provider-start-decision-publication-plan.v2",
  );
  assert.notEqual(variants[0].kind, variants[1].kind);
  assert.notEqual(variants[0].digest, variants[1].digest);

  const deniedCapabilities = [
    "cleanup_authorized",
    "cross_namespace_atomic_reservation",
    "effect_execution_authorized",
    "environment_publication_authorized",
    "evidence_publication_authorized",
    "finalization_authorized",
    "lifecycle_exposed",
    "process_signal_authorized",
    "process_spawn_authorized",
    "process_start_authorized",
    "provider_adapter_execution_authorized",
    "provider_artifact_publication_authorized",
    "provider_recovery_authorized",
    "provider_root_mutation_authorized",
    "receipt_publication_authorized",
    "runtime_publication_authorized",
  ];
  for (const value of variants) {
    assert.equal(value.digest, value.expectedDigest);
    assert.equal(value.digest, digest(value.contract));
    assert.equal(value.contract.action, COLIMA_LIVE_PROVIDER_START_DECISION_ACTION);
    assert.equal(value.contract.operation_kind, value.kind);
    assert.equal(value.contract.evidence_class, value.evidenceClass);
    assert.equal(value.contract.provider_class, "colima-vz-docker-live");
    assert.equal(
      value.contract.target_provider_intent_operation_kind,
      value.intentKind,
    );
    assert.equal(
      value.contract.target_provider_intent_operation_contract_sha256,
      value.intentContractSha256,
    );
    assert.equal(
      value.contract.target_provider_intent_completion_schema,
      value.intentCompletionSchema,
    );
    assert.equal(
      value.contract.target_provider_intent_state_integration,
      "mutation-journal-v5-inert-intent-only",
    );
    assert.equal(
      value.contract.state_process_start_decision_publication_authorized,
      true,
    );
    assert.equal(
      value.contract.state_integration,
      "mutation-journal-v5-inert-start-decision-only",
    );
    assert.equal(value.contract.future_effect_fresh_admission_required, true);
    assert.equal(
      value.contract.decision,
      "requested-not-executed-not-authorized",
    );
    assert.equal(value.contract.process_state, "not-reached");
    assert.equal(value.contract.recovery_disposition, "aborted-before-effect-only");
    for (const field of deniedCapabilities) assert.equal(value.contract[field], false);
    assertRecursivelyFrozen(value.contract);

    const tuple = {
      action: value.contract.action,
      operation_contract_sha256: value.digest,
      operation_kind: value.kind,
      provider_class: value.contract.provider_class,
    };
    for (const operation of [
      resolveProviderAdapter,
      authorizeProviderAdapterPlanning,
      authorizeProviderAdapter,
    ]) {
      assert.throws(
        () => operation(tuple),
        (error) => {
          assert.ok(error instanceof ProviderAdapterRegistryFailure);
          assert.equal(error.exitStatus, 69);
          return true;
        },
      );
    }
  }
});

test("the completed intent projection is exact, source-bound and immutable", () => {
  for (const fixtureOnly of [false, true]) {
    const fixture = cleanEngineLiveProviderStartDecisionSourceFixture(fixtureOnly);
    const projection = fixture.completedIntentProjection;
    assert.equal(
      validateColimaLiveCompletedProviderIntentProjectionStructure(
        projection,
        fixture.source,
      ),
      projection,
    );
    assert.deepEqual(Object.keys(projection).sort(), [
      "close_authority",
      "completed_plan_projection_sha256",
      "evidence_class",
      "provider_intent_close_sha256",
      "provider_intent_operation_contract_sha256",
      "provider_intent_operation_kind",
      "provider_intent_publication_plan_sha256",
      "provider_intent_slot_sha256",
      "schema",
    ]);
    assert.equal(projection.close_authority, "owner");
    assert.notEqual(
      projection.provider_intent_slot_sha256,
      projection.provider_intent_close_sha256,
    );
    assertRecursivelyFrozen(projection);
    assert.deepEqual(
      buildColimaLiveCompletedProviderIntentProjectionStructure(fixture.source),
      projection,
    );

    for (const field of [
      "completed_plan_projection_sha256",
      "provider_intent_close_sha256",
      "provider_intent_operation_contract_sha256",
      "provider_intent_publication_plan_sha256",
      "provider_intent_slot_sha256",
    ]) {
      for (const replacement of ["0".repeat(64), "F".repeat(64), "f".repeat(63)]) {
        expectRefusal(() =>
          validateColimaLiveCompletedProviderIntentProjectionStructure(
            { ...projection, [field]: replacement },
            fixture.source,
          ),
        );
      }
    }
    expectRefusal(() =>
      validateColimaLiveCompletedProviderIntentProjectionStructure(
        {
          ...projection,
          provider_intent_close_sha256: projection.provider_intent_slot_sha256,
          provider_intent_slot_sha256: projection.provider_intent_close_sha256,
        },
        fixture.source,
      ),
    );
    const missing = { ...projection };
    delete missing.close_authority;
    expectRefusal(() =>
      validateColimaLiveCompletedProviderIntentProjectionStructure(
        missing,
        fixture.source,
      ),
    );
    expectRefusal(() =>
      validateColimaLiveCompletedProviderIntentProjectionStructure(
        { ...projection, unreviewed: true },
        fixture.source,
      ),
    );
  }

  const production = cleanEngineLiveProviderStartDecisionSourceFixture(false);
  const fixture = cleanEngineLiveProviderStartDecisionSourceFixture(true);
  expectRefusal(() =>
    validateColimaLiveCompletedProviderIntentProjectionStructure(
      production.completedIntentProjection,
      fixture.source,
    ),
  );
});

test("fresh admission produces only an inert decision for pristine namespaces", () => {
  for (const fixtureOnly of [false, true]) {
    const fixture = cleanEngineLiveProviderStartFreshAdmissionFixture(fixtureOnly);
    const { admission } = fixture;
    const candidate = admission.process_start_decision_candidate;
    assert.equal(
      validateColimaLiveProviderStartFreshAdmissionStructure(
        admission,
        fixture.source,
      ),
      admission,
    );
    assert.equal(
      validateColimaLiveProviderStartDecisionCandidateStructure(
        candidate,
        candidateExpectation(fixture),
      ),
      candidate,
    );
    assert.equal(
      admission.authority,
      fixtureOnly
        ? "fixture-only-point-in-time-not-process-start-authority"
        : "point-in-time-not-process-start-authority",
    );
    assert.equal(candidate.effect_name, "provider-process-start");
    assert.equal(candidate.decision, "requested-not-executed-not-authorized");
    assert.equal(candidate.process_state, "not-reached");
    assert.equal(candidate.process_start_authorized, false);
    assert.equal(candidate.future_effect_fresh_admission_required, true);
    assert.equal(candidate.cross_namespace_atomic_reservation, false);
    assert.equal(
      candidate.completed_intent_projection_sha256,
      digest(fixture.completedIntentProjection),
    );
    assert.equal(candidate.root_observation_sha256, digest(fixture.rootObservation));
    assertRecursivelyFrozen(admission);

    const rebuiltCandidate = buildColimaLiveProviderStartDecisionCandidateStructure(
      candidateExpectation(fixture),
    );
    assert.deepEqual(rebuiltCandidate, candidate);
    assert.deepEqual(
      buildColimaLiveProviderStartFreshAdmissionStructure(
        candidateExpectation(fixture),
      ),
      admission,
    );

    const candidateMutations = [
      { ...candidate, decision: "start" },
      { ...candidate, decision: "authorized" },
      { ...candidate, process_state: "running" },
      { ...candidate, process_start_authorized: true },
      { ...candidate, future_effect_fresh_admission_required: false },
      { ...candidate, cross_namespace_atomic_reservation: true },
      { ...candidate, completed_intent_projection_sha256: "f".repeat(64) },
      { ...candidate, root_observation_sha256: "f".repeat(64) },
      { ...candidate, supervisor_process_group_label: "1234" },
      { ...candidate, pid: 1234 },
      { ...candidate, command: ["colima", "start"] },
    ];
    for (const changed of candidateMutations) {
      expectRefusal(() =>
        validateColimaLiveProviderStartDecisionCandidateStructure(
          changed,
          candidateExpectation(fixture),
        ),
      );
    }
  }
});

test("fresh namespace and admission shapes fail closed on every authority binding", () => {
  for (const fixtureOnly of [false, true]) {
    const fixture = cleanEngineLiveProviderStartFreshAdmissionFixture(fixtureOnly);
    const root = fixture.rootObservation;
    const rootMutations = [
      { ...root, schema: "other" },
      {
        ...root,
        evidence_class: fixtureOnly ? "production-pinned" : "fixture-only",
      },
      {
        ...root,
        planned_names: { ...root.planned_names, provider_profile: "other" },
      },
      {
        ...root,
        planned_names: { ...root.planned_names, lima_instance: "other" },
      },
      { ...root, requirements_sha256: "f".repeat(64) },
      { ...root, preparation_observation_sha256: "f".repeat(64) },
      { ...root, root_observations: [...root.root_observations].reverse() },
      {
        ...root,
        root_observations: [
          {
            ...root.root_observations[0],
            role: COLIMA_LIVE_MUTATION_SURFACE_ROLES[1],
          },
          ...root.root_observations.slice(1),
        ],
      },
      { ...root, root_set_disposition: "foreign-collision" },
      {
        ...root,
        root_observations: [
          {
            ...root.root_observations[0],
            namespace_identity_hmac_sha256: ["1".repeat(64)],
          },
          ...root.root_observations.slice(1),
        ],
      },
      {
        ...root,
        root_observations: [
          {
            ...root.root_observations[0],
            observed_entry_set_hmac_sha256: ["6".repeat(64)],
          },
          ...root.root_observations.slice(1),
        ],
      },
      { ...root, ignored: true },
    ];
    const missingRoot = { ...root };
    delete missingRoot.requirements_sha256;
    rootMutations.push(missingRoot);
    for (const changedRoot of rootMutations) {
      expectRefusal(() =>
        buildColimaLiveProviderStartFreshAdmissionStructure({
          completedIntentProjection: fixture.completedIntentProjection,
          rootObservation: changedRoot,
          source: fixture.source,
        }),
      );
    }

    const admissionMutations = [
      { ...fixture.admission, authority: "process-start-authority" },
      { ...fixture.admission, schema: "other" },
      { ...fixture.admission, supervisor_process_group_label: "1234" },
      { ...fixture.admission, process_start_decision_candidate: null },
      { ...fixture.admission, ignored: true },
    ];
    const missingAdmission = { ...fixture.admission };
    delete missingAdmission.authority;
    admissionMutations.push(missingAdmission);
    for (const admission of admissionMutations) {
      expectRefusal(() =>
        validateColimaLiveProviderStartFreshAdmissionStructure(
          admission,
          fixture.source,
        ),
      );
    }
  }
});

test("a foreign root collision yields no decision and cannot become a plan", () => {
  for (const fixtureOnly of [false, true]) {
    for (const collisionRole of [
      ...COLIMA_LIVE_MUTATION_SURFACE_ROLES,
      [...COLIMA_LIVE_MUTATION_SURFACE_ROLES],
    ]) {
      const fixture =
        cleanEngineLiveProviderStartDecisionSourceFixture(fixtureOnly);
      const rootObservation =
        cleanEngineLiveProviderStartFreshRootObservationFixture(
          fixture.source,
          collisionRole,
        );
      const admission = buildColimaLiveProviderStartFreshAdmissionStructure({
        completedIntentProjection: fixture.completedIntentProjection,
        rootObservation,
        source: fixture.source,
      });
      assert.equal(
        admission.root_observation.root_set_disposition,
        "foreign-collision",
      );
      assert.equal(admission.process_start_decision_candidate, null);
      assert.equal(
        validateColimaLiveProviderStartFreshAdmissionStructure(
          admission,
          fixture.source,
        ),
        admission,
      );
      expectRefusal(
        () =>
          buildColimaLiveProviderStartDecisionPublicationPlan({
            admission,
            source: fixture.source,
          }),
        73,
      );
      expectRefusal(() =>
        validateColimaLiveProviderStartFreshAdmissionStructure(
          {
            ...admission,
            process_start_decision_candidate: {
              schema: "untrusted",
            },
          },
          fixture.source,
        ),
      );
      const nonStringCollision = structuredClone(rootObservation);
      nonStringCollision.root_observations.find(
        (root) => root.disposition === "foreign-collision",
      ).observed_entry_set_hmac_sha256 = ["6".repeat(64)];
      expectRefusal(() =>
        buildColimaLiveProviderStartFreshAdmissionStructure({
          completedIntentProjection: fixture.completedIntentProjection,
          rootObservation: nonStringCollision,
          source: fixture.source,
        }),
      );
    }
  }
});

test("publication plans and completion structures are exact and variant-bound", () => {
  const built = [];
  for (const fixtureOnly of [false, true]) {
    const fixture = cleanEngineLiveProviderStartFreshAdmissionFixture(fixtureOnly);
    const publicationPlan = buildColimaLiveProviderStartDecisionPublicationPlan({
      admission: fixture.admission,
      source: fixture.source,
    });
    built.push({ fixture, publicationPlan });
    assert.equal(
      validateColimaLiveProviderStartDecisionPublicationPlan(
        publicationPlan,
        fixture.source,
      ),
      publicationPlan,
    );
    assert.equal(
      publicationPlan.schema,
      fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA
        : COLIMA_LIVE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
    );
    assert.equal(publicationPlan.admission_sha256, digest(fixture.admission));
    assert.equal(
      publicationPlan.completed_intent_projection_sha256,
      digest(fixture.completedIntentProjection),
    );
    assert.equal(
      publicationPlan.root_observation_sha256,
      digest(fixture.rootObservation),
    );
    assert.equal(
      publicationPlan.decision_candidate_sha256,
      digest(fixture.admission.process_start_decision_candidate),
    );
    assert.equal(
      publicationPlan.publication_claim,
      "state-owner-reconstructs-equal-fresh-admission-at-each-publication-boundary",
    );
    assert.equal(
      publicationPlan.state_integration,
      "mutation-journal-v5-inert-start-decision-only",
    );
    assertRecursivelyFrozen(publicationPlan);
    expectRefusal(() =>
      validateColimaLiveProviderStartDecisionPublicationPlan(
        {
          ...publicationPlan,
          operation_contract_sha256: fixtureOnly
            ? "83866dac58880b4d0bf69361cb23a9e34df2bd624687d0cf8e297867e3acef7c"
            : "2d9033bd071e24125e61533ec75500f6bc6d909226a0209be95ecd80df407cb5",
        },
        fixture.source,
      ),
    );

    for (const field of Object.keys(publicationPlan)) {
      const changed = {
        ...publicationPlan,
        [field]:
          field === "admission"
            ? { ...publicationPlan.admission, authority: "effect-authority" }
            : "f".repeat(64),
      };
      expectRefusal(() =>
        validateColimaLiveProviderStartDecisionPublicationPlan(
          changed,
          fixture.source,
        ),
      );
    }
    const missingPlan = { ...publicationPlan };
    delete missingPlan.root_observation_sha256;
    expectRefusal(() =>
      validateColimaLiveProviderStartDecisionPublicationPlan(
        missingPlan,
        fixture.source,
      ),
    );
    expectRefusal(() =>
      validateColimaLiveProviderStartDecisionPublicationPlan(
        { ...publicationPlan, ignored: true },
        fixture.source,
      ),
    );

    const expected = {
      closeSha256: "8".repeat(64),
      publicationPlan,
      slotSha256: "9".repeat(64),
      source: fixture.source,
    };
    const completion = buildColimaLiveProviderStartDecisionCompletion(expected);
    assert.equal(
      validateColimaLiveProviderStartDecisionCompletion(completion, expected),
      completion,
    );
    assert.equal(
      completion.schema,
      fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA
        : COLIMA_LIVE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
    );
    assert.equal(
      completion.authority,
      fixtureOnly
        ? "fixture-only-durable-inert-process-start-decision-not-effect-authority"
        : "durable-inert-process-start-decision-not-effect-authority",
    );
    assert.equal(completion.decision_slot_sha256, "9".repeat(64));
    assert.equal(completion.decision_close_sha256, "8".repeat(64));
    assertRecursivelyFrozen(completion);

    for (const field of Object.keys(completion)) {
      const changed = {
        ...completion,
        [field]: field === "authority" ? "effect-authority" : "f".repeat(64),
      };
      expectRefusal(() =>
        validateColimaLiveProviderStartDecisionCompletion(changed, expected),
      );
    }
    const missing = { ...completion };
    delete missing.decision_close_sha256;
    expectRefusal(() =>
      validateColimaLiveProviderStartDecisionCompletion(missing, expected),
    );
    expectRefusal(() =>
      validateColimaLiveProviderStartDecisionCompletion(
        { ...completion, ignored: true },
        expected,
      ),
    );
    expectRefusal(() =>
      validateColimaLiveProviderStartDecisionCompletion(completion, {
        ...expected,
        closeSha256: expected.slotSha256,
        slotSha256: expected.closeSha256,
      }),
    );
  }
  expectRefusal(() =>
    validateColimaLiveProviderStartDecisionPublicationPlan(
      built[0].publicationPlan,
      built[1].fixture.source,
    ),
  );
});

test("valid alternate source values cannot replay a historical projection", () => {
  const original = cleanEngineLiveProviderStartDecisionSourceFixture(false);
  const changedIntentCompletion = {
    ...original.source.intentCompletion,
    intent_close_sha256: "a".repeat(64),
    intent_slot_sha256: "b".repeat(64),
  };
  const changedSource = {
    ...original.source,
    intentCompletion: changedIntentCompletion,
  };
  const changedProjection =
    buildColimaLiveCompletedProviderIntentProjectionStructure(changedSource);
  assert.notEqual(digest(changedProjection), digest(original.completedIntentProjection));
  expectRefusal(() =>
    validateColimaLiveCompletedProviderIntentProjectionStructure(
      original.completedIntentProjection,
      changedSource,
    ),
  );
  expectRefusal(() =>
    validateColimaLiveCompletedProviderIntentProjectionStructure(
      changedProjection,
      original.source,
    ),
  );

  const fixture = cleanEngineLiveProviderStartFreshAdmissionFixture(false);
  const publicationPlan = buildColimaLiveProviderStartDecisionPublicationPlan({
    admission: fixture.admission,
    source: fixture.source,
  });
  const changedCandidate = {
    ...fixture.admission.process_start_decision_candidate,
    decision: "start",
  };
  const changedAdmission = {
    ...fixture.admission,
    process_start_decision_candidate: changedCandidate,
  };
  const validlyRehashed = {
    ...publicationPlan,
    admission: changedAdmission,
    admission_sha256: digest(changedAdmission),
    decision_candidate_sha256: digest(changedCandidate),
  };
  expectRefusal(() =>
    validateColimaLiveProviderStartDecisionPublicationPlan(
      validlyRehashed,
      fixture.source,
    ),
  );
});

test("builder and validator inputs are closed", () => {
  const fixture = cleanEngineLiveProviderStartFreshAdmissionFixture(false);
  const publicationPlan = buildColimaLiveProviderStartDecisionPublicationPlan({
    admission: fixture.admission,
    source: fixture.source,
  });
  for (const value of [undefined, null, [], {}]) {
    expectRefusal(() => buildColimaLiveCompletedProviderIntentProjectionStructure(value));
    expectRefusal(() => buildColimaLiveProviderStartDecisionCandidateStructure(value));
    expectRefusal(() => buildColimaLiveProviderStartFreshAdmissionStructure(value));
    expectRefusal(() => buildColimaLiveProviderStartDecisionPublicationPlan(value));
    expectRefusal(() => buildColimaLiveProviderStartDecisionCompletion(value));
  }
  expectRefusal(() =>
    buildColimaLiveProviderStartDecisionPublicationPlan({
      admission: fixture.admission,
      source: fixture.source,
      unreviewed: true,
    }),
  );
  expectRefusal(
    () =>
      validateColimaLiveProviderStartDecisionCompletion(
        buildColimaLiveProviderStartDecisionCompletion({
          closeSha256: "8".repeat(64),
          publicationPlan,
          slotSha256: "9".repeat(64),
          source: fixture.source,
        }),
        {
          closeSha256: "8".repeat(64),
          publicationPlan,
          slotSha256: "9".repeat(64),
          source: fixture.source,
          unreviewed: true,
        },
      ),
    70,
  );
});

test("the structural boundary directly owns no effect seam and leaves start guards pinned", () => {
  const fixture = cleanEngineLiveProviderStartFreshAdmissionFixture(false);
  const serialized = Buffer.concat([
    liveProviderStartDecisionBytes(fixture.completedIntentProjection),
    liveProviderStartDecisionBytes(fixture.admission),
  ]).toString("utf8");
  for (const forbidden of [
    "/Users/",
    "/home/",
    "background-provider-start-decision",
    "command",
    "controlled-background",
    "docker-context",
    "environment",
    "executable",
    "guest-engine",
    "host-agent",
    "password",
    "pgid",
    "provider-start-decision.json",
    "provider_identity",
    "runtime_identity",
    "secret",
    "socket",
    "token",
  ]) {
    assert.equal(serialized.includes(forbidden), false, forbidden);
  }

  const source = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-live-provider-start-decision.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(source, /node:(?:child_process|fs|http|https|net|tls)/u);
  assert.doesNotMatch(
    source,
    /clean-engine-(?:receipts|state|provider-process-contract)\.mjs/u,
  );
  assert.doesNotMatch(source, /scripts\/fixtures/u);
  assert.doesNotMatch(
    source,
    /\b(?:appendFile|chmod|link|mkdir|open|rename|rm|rmdir|spawn|unlink|writeFile)Sync\s*\(/u,
  );
  assert.doesNotMatch(source, /export function [A-Za-z0-9_]*(?:execute|launch|publish|recover)/iu);

  assert.equal(
    COLIMA_LIVE_PREPARATION_CONTRACT_SHA256,
    "fb364b1cd89e7534b10dbd69d1092c93e64d17746b11692dec3b4252f83cbf51",
  );
  assert.equal(
    CONTROLLED_BACKGROUND_PROVIDER_CONTRACT_SHA256,
    "8d91f1e7023f4c32c00f2a2d7a75c59809ceb02cfe7588ba4a44cba990646992",
  );
  assert.throws(
    () =>
      authorizeColimaLiveStart(COLIMA_LIVE_PREPARATION_CONTRACT, {
        architecture: "arm64",
        platform: "darwin",
      }),
    (error) => {
      assert.ok(error instanceof ProviderProcessContractFailure);
      assert.equal(error.exitStatus, 69);
      assert.equal(
        error.message,
        "Colima live start remains blocked by the unresolved toolchain closure",
      );
      return true;
    },
  );

  const state = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-state.mjs", import.meta.url),
    "utf8",
  );
  assert.match(
    state,
    /observeColimaLiveProviderStartFreshAdmissionForExecutor/u,
  );
  assert.match(state, /COLIMA_LIVE_PROVIDER_START_DECISION_ACTION/u);
  assert.match(state, /publishColimaLiveProviderStartDecisionForExecutor/u);
  assert.match(state, /recoverLiveProviderStartDecisionForExecutor/u);
  assert.doesNotMatch(state, /executeColimaLiveProviderStartDecision/u);
  assert.doesNotMatch(state, /case "provider-start-decision"/u);
  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /provider-start-decision/u);
});
