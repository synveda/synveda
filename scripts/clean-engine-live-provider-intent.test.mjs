import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  LiveProviderIntentFailure,
  buildColimaLiveEffectIntentCandidateStructure,
  buildColimaLiveEmptyPreEffectPrefixStructure,
  buildColimaLivePlanCompletionProjectionStructure,
  liveProviderIntentBytes,
  liveProviderIntentDigest,
  validateColimaLiveEffectIntentCandidateStructure,
  validateColimaLiveEmptyPreEffectPrefixStructure,
  validateColimaLivePlanCompletionProjectionStructure,
} from "../deploy/compose/scripts/clean-engine-live-provider-intent.mjs";
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

function fixture() {
  const { completionProjection, operationPlan } =
    cleanEngineLivePlanCompletionProjectionFixture();
  const intent = cleanEngineLiveEffectIntentFixture(
    completionProjection,
    operationPlan,
  );
  return { completionProjection, intent, operationPlan };
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
    /if \(initial\.liveProviderPlan !== undefined\) \{\s*fail\("live provider execution remains disabled after state planning", 73\);/u,
  );

  const receipts = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-receipts.mjs", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(receipts, /colima-live-effect-intent/u);

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
