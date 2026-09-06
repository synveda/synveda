import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
  COLIMA_LIVE_PROVIDER_INTENT_ACTION,
  COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
  LiveProviderIntentFailure,
  buildColimaLiveEffectIntentCandidateStructure,
  buildColimaLiveEmptyPreEffectPrefixStructure,
  buildColimaLivePlanCompletionProjectionStructure,
  buildColimaLiveProviderIntentCompletion,
  buildColimaLiveProviderIntentPublicationPlan,
  liveProviderIntentBytes,
  liveProviderIntentDigest,
  validateColimaLiveEffectIntentCandidateStructure,
  validateColimaLiveEmptyPreEffectPrefixStructure,
  validateColimaLivePlanCompletionProjectionStructure,
  validateColimaLiveProviderIntentCompletion,
  validateColimaLiveProviderIntentPublicationPlan,
} from "../deploy/compose/scripts/clean-engine-live-provider-intent.mjs";
import {
  ProviderAdapterRegistryFailure,
  authorizeProviderAdapterPlanning,
  resolveProviderAdapter,
} from "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs";
import {
  cleanEngineLiveEffectIntentFixture,
  cleanEngineLiveEmptyPreEffectPrefixFixture,
  cleanEngineLivePlanCompletionProjectionFixture,
} from "./fixtures/clean-engine-live-provider-intent-fixture.mjs";

function digest(value) {
  return liveProviderIntentDigest(liveProviderIntentBytes(value));
}

function expectRefusal(operation, exitStatus) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof LiveProviderIntentFailure);
    if (exitStatus !== undefined) assert.equal(error.exitStatus, exitStatus);
    return true;
  });
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function fixture() {
  const { completionProjection, operationPlan } =
    cleanEngineLivePlanCompletionProjectionFixture();
  const intent = cleanEngineLiveEffectIntentFixture(
    completionProjection,
    operationPlan,
  );
  return { completionProjection, intent, operationPlan };
}

function admissionFixture(fixtureOnly = false) {
  const { completionProjection, intent, operationPlan } = fixture();
  const preEffectPrefix = buildColimaLiveEmptyPreEffectPrefixStructure({
    completionProjection,
    intent,
    operationPlan,
  });
  return {
    admission: {
      authority: fixtureOnly
        ? "fixture-only-point-in-time-not-effect-authority"
        : "point-in-time-not-effect-authority",
      completion_projection: completionProjection,
      intent_candidate: intent,
      pre_effect_prefix: preEffectPrefix,
      root_observation: {
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
        root_observations: [
          {
            disposition: "observed-absent",
            parent_identity_hmac_sha256: "1".repeat(64),
            role: "colima-profile-root",
            target_entry_identity_hmac_sha256: "0".repeat(64),
            target_path_hmac_sha256: "2".repeat(64),
          },
          {
            disposition: "observed-absent",
            parent_identity_hmac_sha256: "3".repeat(64),
            role: "lima-instance-root",
            target_entry_identity_hmac_sha256: "0".repeat(64),
            target_path_hmac_sha256: "4".repeat(64),
          },
        ],
        root_set_disposition: "observed-absent",
        schema: fixtureOnly
          ? "synveda.clean-engine.colima-live-fixture-pre-effect-root-observation.v1"
          : "synveda.clean-engine.colima-live-pre-effect-root-observation.v1",
      },
      schema: fixtureOnly
        ? "synveda.clean-engine.colima-live-fixture-pre-effect-admission.v1"
        : "synveda.clean-engine.colima-live-pre-effect-admission.v1",
      supervisor_process_group_label:
        `sv-c45-colima-pg-${operationPlan.fixture_id}-` +
        completionProjection.plan_slot_sha256.slice(0, 12),
    },
    operationPlan,
  };
}

test("the completed-plan projection is minimal, exact and immutable", () => {
  const { completionProjection } = fixture();
  assert.equal(
    validateColimaLivePlanCompletionProjectionStructure(completionProjection),
    completionProjection,
  );
  assert.deepEqual(Object.keys(completionProjection).sort(), [
    "operation_plan_sha256",
    "plan_close_sha256",
    "plan_slot_sha256",
    "preparation_observation_sha256",
    "schema",
  ]);
  assert.ok(Object.isFrozen(completionProjection));

  for (const field of [
    "operation_plan_sha256",
    "plan_close_sha256",
    "plan_slot_sha256",
    "preparation_observation_sha256",
  ]) {
    for (const replacement of ["0".repeat(64), "F".repeat(64), "f".repeat(63)]) {
      const changed = { ...completionProjection, [field]: replacement };
      expectRefusal(() =>
        validateColimaLivePlanCompletionProjectionStructure(changed),
      );
    }
  }
  const missing = { ...completionProjection };
  delete missing.plan_close_sha256;
  expectRefusal(() => validateColimaLivePlanCompletionProjectionStructure(missing));
  expectRefusal(() =>
    validateColimaLivePlanCompletionProjectionStructure({
      ...completionProjection,
      unreviewed: true,
    }),
  );
});

test("the intent candidate binds the plan and requests no authority", () => {
  const { completionProjection, intent, operationPlan } = fixture();
  assert.equal(
    validateColimaLiveEffectIntentCandidateStructure(
      intent,
      completionProjection,
      operationPlan,
    ),
    intent,
  );
  assert.deepEqual(Object.keys(intent).sort(), [
    "completed_plan_projection_sha256",
    "effect_authorization",
    "effect_name",
    "preparation_observation_sha256",
    "schema",
  ]);
  assert.equal(intent.effect_name, "provider-create");
  assert.equal(intent.effect_authorization, "requested-not-authorized");
  assert.ok(Object.isFrozen(intent));
  const repeated = buildColimaLiveEffectIntentCandidateStructure({
    completionProjection,
    operationPlan,
  });
  assert.deepEqual(
    liveProviderIntentBytes(repeated),
    liveProviderIntentBytes(intent),
  );

  const mutations = [
    (value) => {
      value.completed_plan_projection_sha256 = "f".repeat(64);
    },
    (value) => {
      value.effect_authorization = "authorized";
    },
    (value) => {
      value.effect_name = "controlled-background-provider-create";
    },
    (value) => {
      value.preparation_observation_sha256 = "f".repeat(64);
    },
    (value) => {
      value.schema = "other";
    },
    (value) => {
      value.unreviewed = true;
    },
  ];
  for (const mutate of mutations) {
    const changed = structuredClone(intent);
    mutate(changed);
    expectRefusal(() =>
      validateColimaLiveEffectIntentCandidateStructure(
        changed,
        completionProjection,
        operationPlan,
      ),
    );
  }

  const changedProjection = {
    ...completionProjection,
    plan_close_sha256: "1".repeat(64),
  };
  expectRefusal(() =>
    validateColimaLiveEffectIntentCandidateStructure(
      intent,
      changedProjection,
      operationPlan,
    ),
  );
  const changedPlan = structuredClone(operationPlan);
  changedPlan.preparation_observation_sha256 = "1".repeat(64);
  expectRefusal(() =>
    validateColimaLiveEffectIntentCandidateStructure(
      intent,
      completionProjection,
      changedPlan,
    ),
  );
});

test("the logical pre-effect prefix is genuinely empty", () => {
  const { completionProjection, intent, operationPlan } = fixture();
  const prefix = buildColimaLiveEmptyPreEffectPrefixStructure({
    completionProjection,
    intent,
    operationPlan,
  });
  assert.deepEqual(prefix, cleanEngineLiveEmptyPreEffectPrefixFixture(intent));
  assert.equal(
    validateColimaLiveEmptyPreEffectPrefixStructure(
      prefix,
      intent,
      completionProjection,
      operationPlan,
    ),
    prefix,
  );
  assert.deepEqual(Object.keys(prefix).sort(), [
    "entry_count",
    "intent_candidate_sha256",
    "schema",
  ]);
  assert.equal(prefix.entry_count, 0);
  assert.equal(prefix.intent_candidate_sha256, digest(intent));
  assert.ok(Object.isFrozen(prefix));

  for (const changed of [
    { ...prefix, entry_count: 1 },
    { ...prefix, intent_candidate_sha256: "0".repeat(64) },
    { ...prefix, schema: "other" },
    { ...prefix, absence: "proved" },
  ]) {
    expectRefusal(() =>
      validateColimaLiveEmptyPreEffectPrefixStructure(
        changed,
        intent,
        completionProjection,
        operationPlan,
      ),
    );
  }
  const repeated = buildColimaLiveEmptyPreEffectPrefixStructure({
    completionProjection,
    intent,
    operationPlan,
  });
  assert.deepEqual(liveProviderIntentBytes(repeated), liveProviderIntentBytes(prefix));
});

test("structural bindings deliberately carry no state or observation provenance", () => {
  const { completionProjection, operationPlan } = fixture();
  const manufactured = buildColimaLivePlanCompletionProjectionStructure({
    operationPlanSha256: completionProjection.operation_plan_sha256,
    planCloseSha256: "1".repeat(64),
    planSlotSha256: "2".repeat(64),
    preparationObservationSha256:
      operationPlan.preparation_observation_sha256,
  });
  const intent = buildColimaLiveEffectIntentCandidateStructure({
    completionProjection: manufactured,
    operationPlan,
  });
  const replayed = JSON.parse(
    liveProviderIntentBytes(manufactured).toString("utf8"),
  );

  assert.equal(
    validateColimaLivePlanCompletionProjectionStructure(replayed),
    replayed,
  );
  assert.equal(intent.effect_authorization, "requested-not-authorized");
  assert.notEqual(Object.isFrozen(replayed), true);
});

test("structural builder inputs are closed", () => {
  const { completionProjection, intent, operationPlan } = fixture();
  for (const value of [undefined, null, [], {}]) {
    expectRefusal(() => buildColimaLivePlanCompletionProjectionStructure(value));
    expectRefusal(() => buildColimaLiveEffectIntentCandidateStructure(value));
    expectRefusal(() => buildColimaLiveEmptyPreEffectPrefixStructure(value));
  }
  expectRefusal(() =>
    buildColimaLivePlanCompletionProjectionStructure({
      operationPlanSha256: completionProjection.operation_plan_sha256,
      planCloseSha256: completionProjection.plan_close_sha256,
      planSlotSha256: completionProjection.plan_slot_sha256,
      preparationObservationSha256:
        completionProjection.preparation_observation_sha256,
      unreviewed: true,
    }),
  );
  expectRefusal(() =>
    buildColimaLiveEffectIntentCandidateStructure({
      completionProjection,
      operationPlan,
      unreviewed: true,
    }),
  );
  expectRefusal(() =>
    buildColimaLiveEmptyPreEffectPrefixStructure({
      completionProjection,
      intent,
      operationPlan,
      unreviewed: true,
    }),
  );
});

test("the three contract values contain no private, live or fake evidence", () => {
  const { completionProjection, intent, operationPlan } = fixture();
  const prefix = buildColimaLiveEmptyPreEffectPrefixStructure({
    completionProjection,
    intent,
    operationPlan,
  });
  const serialized = Buffer.concat([
    liveProviderIntentBytes(completionProjection),
    liveProviderIntentBytes(intent),
    liveProviderIntentBytes(prefix),
  ]).toString("utf8");
  for (const forbidden of [
    "/Users/",
    "/home/",
    "HOME",
    "absence",
    "binding_key",
    "command",
    "controlled-background",
    "deterministic-fake",
    "DOCKER_CONFIG",
    "host_agent",
    "not-reached",
    "password",
    "preexisting_resource",
    "provider-create-intent",
    "provider_resource",
    "secret",
    "socket",
    "TMPDIR",
    "token",
  ]) {
    assert.equal(serialized.includes(forbidden), false, forbidden);
  }
});

test("intent publication has distinct inert production and fixture contracts", () => {
  const variants = [
    {
      contract: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT,
      digest: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
      evidenceClass: "production-pinned",
      kind: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
      publicationPlanSchema:
        COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
      schema:
        "synveda.clean-engine.colima-live-provider-intent-operation-contract.v1",
    },
    {
      contract: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT,
      digest:
        COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
      evidenceClass: "fixture-only",
      kind: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
      publicationPlanSchema:
        COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
      schema:
        "synveda.clean-engine.colima-live-fixture-provider-intent-operation-contract.v1",
    },
  ];
  assert.equal(COLIMA_LIVE_PROVIDER_INTENT_ACTION, "provider-intent");
  assert.notEqual(variants[0].kind, variants[1].kind);
  assert.notEqual(variants[0].digest, variants[1].digest);
  assert.equal(
    variants[0].digest,
    "8cad231ffcc14cd58a90df10faee867bcb8a752d46772e1ccedc666d744648ee",
  );
  assert.equal(
    variants[1].digest,
    "d8269df82f7d10c9bfe02f36a9e42d05138d4dc1798a08bcee1c023b2eedfcc2",
  );
  for (const value of variants) {
    assert.deepEqual(value.contract, {
      action: COLIMA_LIVE_PROVIDER_INTENT_ACTION,
      cleanup_authorized: false,
      effect_execution_authorized: false,
      evidence_class: value.evidenceClass,
      finalization_authorized: false,
      lifecycle_exposed: false,
      operation_kind: value.kind,
      provider_class: "colima-vz-docker-live",
      provider_recovery_authorized: false,
      publication_plan_schema: value.publicationPlanSchema,
      receipt_publication_authorized: false,
      recovery_disposition: "aborted-before-effect-only",
      schema: value.schema,
      state_integration: "mutation-journal-v5-inert-intent-only",
      state_intent_publication_authorized: true,
      target_create_operation_contract_sha256:
        "13a87072a49103db0b3c4f36b64b8fbd0d74bd794c4a359fb77b41184ee289a7",
      target_create_operation_kind: "colima-vz-docker-live-create-v1",
      target_provider_plan_schema:
        "synveda.clean-engine.colima-live-provider-operation-plan.v1",
      target_provider_plan_state_integration: "mutation-journal-v5-plan-only",
    });
    assert.equal(value.digest, digest(value.contract));
    assertRecursivelyFrozen(value.contract);

    const tuple = {
      action: value.contract.action,
      operation_contract_sha256: value.digest,
      operation_kind: value.kind,
      provider_class: value.contract.provider_class,
    };
    for (const operation of [resolveProviderAdapter, authorizeProviderAdapterPlanning]) {
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

test("publication plans bind all three observations and keep fixtures distinct", () => {
  const built = [];
  for (const fixtureOnly of [false, true]) {
    const { admission, operationPlan } = admissionFixture(fixtureOnly);
    const publicationPlan = buildColimaLiveProviderIntentPublicationPlan({
      admission,
      fixtureOnly,
      operationPlan,
    });
    built.push(publicationPlan);
    assert.equal(
      validateColimaLiveProviderIntentPublicationPlan(publicationPlan, {
        fixtureOnly,
      }),
      publicationPlan,
    );
    assert.equal(
      publicationPlan.schema,
      fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA
        : COLIMA_LIVE_PROVIDER_INTENT_PUBLICATION_PLAN_SCHEMA,
    );
    assert.equal(
      publicationPlan.publication_claim,
      "canonical-root-observation-equal-before-slot-link-after-slot-acquisition-before-close-link",
    );
    assert.equal(publicationPlan.admission_sha256, digest(admission));
    assert.equal(
      publicationPlan.root_observation_sha256,
      digest(admission.root_observation),
    );
    assert.equal(
      publicationPlan.provider_operation_plan_sha256,
      digest(operationPlan),
    );
    assertRecursivelyFrozen(publicationPlan);
    const completion = buildColimaLiveProviderIntentCompletion({
      closeSha256: "6".repeat(64),
      fixtureOnly,
      publicationPlan,
      slotSha256: "7".repeat(64),
    });
    const expectedCompletion = {
      authority: fixtureOnly
        ? "fixture-only-durable-inert-intent-not-effect-authority"
        : "durable-inert-intent-not-effect-authority",
      completed_plan_projection_sha256: digest(
        publicationPlan.admission.completion_projection,
      ),
      evidence_class: fixtureOnly ? "fixture-only" : "production-pinned",
      intent_close_sha256: "6".repeat(64),
      intent_publication_plan_sha256: digest(publicationPlan),
      intent_slot_sha256: "7".repeat(64),
      schema: fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA
        : COLIMA_LIVE_PROVIDER_INTENT_COMPLETION_SCHEMA,
    };
    assert.deepEqual(completion, expectedCompletion);
    const completionExpectation = {
      closeSha256: "6".repeat(64),
      fixtureOnly,
      publicationPlan,
      slotSha256: "7".repeat(64),
    };
    assert.equal(
      validateColimaLiveProviderIntentCompletion(
        completion,
        completionExpectation,
      ),
      completion,
    );
    assertRecursivelyFrozen(completion);
    for (const changed of [
      { ...completion, ignored: true },
      { ...completion, schema: "synveda.clean-engine.invalid-completion.v1" },
      { ...completion, authority: "effect-authority" },
      { ...completion, evidence_class: "unreviewed" },
      { ...completion, intent_close_sha256: "7".repeat(64) },
      { ...completion, intent_slot_sha256: "6".repeat(64) },
      { ...completion, completed_plan_projection_sha256: "f".repeat(64) },
      { ...completion, intent_publication_plan_sha256: "f".repeat(64) },
    ]) {
      expectRefusal(() =>
        validateColimaLiveProviderIntentCompletion(
          changed,
          completionExpectation,
        ),
      );
    }
    const missing = { ...completion };
    delete missing.intent_close_sha256;
    expectRefusal(() =>
      validateColimaLiveProviderIntentCompletion(missing, completionExpectation),
    );
    expectRefusal(() =>
      validateColimaLiveProviderIntentCompletion(completion, {
        ...completionExpectation,
        closeSha256: "7".repeat(64),
        slotSha256: "6".repeat(64),
      }),
    );
    expectRefusal(
      () =>
        validateColimaLiveProviderIntentCompletion(completion, {
          ...completionExpectation,
          unreviewed: true,
        }),
      70,
    );
    expectRefusal(
      () =>
        validateColimaLiveProviderIntentCompletion(completion, {
          ...completionExpectation,
          fixtureOnly: fixtureOnly ? "true" : "false",
        }),
      70,
    );
  }
  assert.notEqual(built[0].schema, built[1].schema);
  assert.notEqual(built[0].operation_kind, built[1].operation_kind);
  expectRefusal(() =>
    validateColimaLiveProviderIntentPublicationPlan(built[0], {
      fixtureOnly: true,
    }),
  );
  for (const options of [
    {},
    { fixtureOnly: "false" },
    { fixtureOnly: false, ignored: true },
  ]) {
    expectRefusal(
      () => validateColimaLiveProviderIntentPublicationPlan(built[0], options),
      70,
    );
  }
  expectRefusal(() =>
    buildColimaLiveProviderIntentPublicationPlan({
      ...admissionFixture(false),
      fixtureOnly: false,
      unreviewed: true,
    }),
  );
  const authorized = structuredClone(built[0]);
  authorized.admission.intent_candidate.effect_authorization = "authorized";
  authorized.admission_sha256 = digest(authorized.admission);
  expectRefusal(() =>
    validateColimaLiveProviderIntentPublicationPlan(authorized),
  );
});

test("the structural module directly owns no mutation, process, network or lifecycle seam", () => {
  const source = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-live-provider-intent.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(source, /node:(?:child_process|fs|http|https|net|tls)/u);
  assert.doesNotMatch(source, /clean-engine-(?:provider-process-contract|receipts|state)\.mjs/u);
  assert.doesNotMatch(source, /scripts\/fixtures/u);
  assert.doesNotMatch(
    source,
    /\b(?:appendFile|chmod|link|mkdir|open|rename|rm|rmdir|spawn|unlink|writeFile)Sync\s*\(/u,
  );
  assert.doesNotMatch(
    source,
    /export function [A-Za-z0-9_]*(?:execute|launch|publish|recover|start)/iu,
  );
  assert.doesNotMatch(source, /clean-engine-colima-live-contract|revalidateColimaLiveObservation/u);

  const state = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-state.mjs", import.meta.url),
    "utf8",
  );
  assert.match(
    state,
    /initial\.liveProviderPlan !== undefined &&[\s\S]*?COLIMA_LIVE_PROVIDER_INTENT_ACTION,[\s\S]*?COLIMA_LIVE_PROVIDER_START_DECISION_ACTION,[\s\S]*?\.has\(action\)[\s\S]*?fail\("live provider execution remains disabled after state planning", 73\);/u,
  );

  const receipts = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-receipts.mjs", import.meta.url),
    "utf8",
  );
  const providerProcess = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  const registry = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  for (const source of [receipts, providerProcess, registry]) {
    assert.doesNotMatch(
      source,
      /colima-live-(?:fixture-)?provider-intent-publication-v1/u,
    );
  }

  const admissionComparisons = state.match(
    /sameLiveProviderIntentValue\(\s*(?:current|acquired|close)Admission,\s*baselineAdmission\s*\)/gu,
  );
  assert.equal(admissionComparisons?.length, 3);

  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /provider-(?:effect-)?intent|colima-live/u);
});
